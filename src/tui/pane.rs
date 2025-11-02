use crate::executor::output::{TaskMessage, TaskOutput, TuiTaskStatus};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};
use std::collections::VecDeque;
use std::time::{Duration, SystemTime};
use tokio::sync::broadcast;

/// Status of a task displayed in a pane
#[derive(Debug, Clone, PartialEq)]
pub enum PaneStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl PaneStatus {
    pub fn symbol(&self) -> &str {
        match self {
            PaneStatus::Pending => "○",
            PaneStatus::Running => "●",
            PaneStatus::Completed => "✓",
            PaneStatus::Failed => "✗",
            PaneStatus::Skipped => "⊘",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            PaneStatus::Pending => Color::Gray,
            PaneStatus::Running => Color::Green,
            PaneStatus::Completed => Color::Green,
            PaneStatus::Failed => Color::Red,
            PaneStatus::Skipped => Color::Yellow,
        }
    }
}

/// Trait for renderable panes
pub trait Pane {
    /// Render the pane to the given area
    fn render(&self, frame: &mut Frame, area: Rect, focused: bool);

    /// Get the pane's identifier (task name)
    fn id(&self) -> &str;

    /// Update pane state (receive from broadcast channel)
    fn update(&mut self);

    /// Get current status
    fn status(&self) -> PaneStatus;

    /// Scroll up
    fn scroll_up(&mut self);

    /// Scroll down
    fn scroll_down(&mut self, visible_height: u16);

    /// Reset scroll to top
    fn reset_scroll(&mut self);
}

/// A pane that displays output from a single task
pub struct TaskPane {
    task_name: String,
    status: PaneStatus,
    output_rx: broadcast::Receiver<TaskOutput>,
    message_rx: Option<broadcast::Receiver<TaskMessage>>,
    output_buffer: VecDeque<String>,
    scroll_offset: u16,
    max_buffer_lines: usize,
    start_time: Option<SystemTime>,
    duration: Option<Duration>,
    auto_scroll: bool,
}

impl TaskPane {
    pub fn new(task_name: String, output_tx: broadcast::Sender<TaskOutput>) -> Self {
        Self {
            task_name: task_name.clone(),
            status: PaneStatus::Pending,
            output_rx: output_tx.subscribe(),
            message_rx: None,
            output_buffer: VecDeque::new(),
            scroll_offset: 0,
            max_buffer_lines: 1000, // Ring buffer
            start_time: None,
            duration: None,
            auto_scroll: true, // Auto-scroll enabled by default
        }
    }

    pub fn set_message_channel(&mut self, message_tx: broadcast::Sender<TaskMessage>) {
        self.message_rx = Some(message_tx.subscribe());
    }

    pub fn set_status(&mut self, status: PaneStatus) {
        self.status = status;
    }

    fn tui_status_to_pane_status(status: &TuiTaskStatus) -> PaneStatus {
        match status {
            TuiTaskStatus::Pending => PaneStatus::Pending,
            TuiTaskStatus::Running => PaneStatus::Running,
            TuiTaskStatus::Completed => PaneStatus::Completed,
            TuiTaskStatus::Failed => PaneStatus::Failed,
            TuiTaskStatus::Skipped => PaneStatus::Skipped,
        }
    }
}

impl Pane for TaskPane {
    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        // Create border with task name, status, and duration
        let mut title = format!(" {} {} ", self.task_name, self.status.symbol());

        if let Some(dur) = &self.duration {
            title.push_str(&format!(" ({:.1}s) ", dur.as_secs_f64()));
        } else if let Some(start) = &self.start_time
            && let Ok(elapsed) = SystemTime::now().duration_since(*start)
        {
            title.push_str(&format!(" ({:.1}s) ", elapsed.as_secs_f64()));
        }

        let border_color = if focused { Color::Yellow } else { self.status.color() };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        // Render output lines with scrolling
        let visible_height = inner_area.height as usize;
        let total_lines = self.output_buffer.len();

        // Auto-scroll to bottom if enabled
        let start_line = if self.auto_scroll && total_lines > visible_height {
            total_lines - visible_height
        } else {
            (self.scroll_offset as usize).min(total_lines.saturating_sub(visible_height))
        };
        let end_line = (start_line + visible_height).min(total_lines);

        let visible_lines: Vec<Line> = self
            .output_buffer
            .iter()
            .skip(start_line)
            .take(end_line - start_line)
            .map(|s| Line::from(s.as_str()))
            .collect();

        let paragraph = Paragraph::new(visible_lines);
        frame.render_widget(paragraph, inner_area);
    }

    fn id(&self) -> &str {
        &self.task_name
    }

    fn update(&mut self) {
        // Process status messages first
        if let Some(rx) = &mut self.message_rx {
            while let Ok(message) = rx.try_recv() {
                match message {
                    TaskMessage::Output(output) => {
                        // Handle output through this channel too
                        if output.task_name == self.task_name {
                            for line in output.content.lines() {
                                self.output_buffer.push_back(line.to_string());
                                if self.output_buffer.len() > self.max_buffer_lines {
                                    self.output_buffer.pop_front();
                                }
                            }
                        }
                    }
                    TaskMessage::StatusChange { task_name, status, .. } => {
                        if task_name == self.task_name {
                            self.status = Self::tui_status_to_pane_status(&status);
                        }
                    }
                    TaskMessage::Started { task_name, timestamp } => {
                        if task_name == self.task_name {
                            self.status = PaneStatus::Running;
                            self.start_time = Some(timestamp);
                        }
                    }
                    TaskMessage::Finished {
                        task_name,
                        status,
                        duration_ms,
                        ..
                    } => {
                        if task_name == self.task_name {
                            self.status = Self::tui_status_to_pane_status(&status);
                            self.duration = Some(Duration::from_millis(duration_ms));
                        }
                    }
                }
            }
        }

        // Non-blocking receive from output broadcast channel
        while let Ok(output) = self.output_rx.try_recv() {
            // Only process output for this task
            if output.task_name == self.task_name {
                // Split content by lines and add to buffer
                for line in output.content.lines() {
                    self.output_buffer.push_back(line.to_string());

                    // Maintain ring buffer
                    if self.output_buffer.len() > self.max_buffer_lines {
                        self.output_buffer.pop_front();
                    }
                }
            }
        }
    }

    fn status(&self) -> PaneStatus {
        self.status.clone()
    }

    fn scroll_up(&mut self) {
        self.auto_scroll = false; // Disable auto-scroll when user manually scrolls
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    fn scroll_down(&mut self, visible_height: u16) {
        let total_lines = self.output_buffer.len() as u16;
        if total_lines > visible_height {
            let max_scroll = total_lines - visible_height;
            if self.scroll_offset < max_scroll {
                self.scroll_offset += 1;
            } else {
                // Re-enable auto-scroll when scrolled to bottom
                self.auto_scroll = true;
            }
        }
    }

    fn reset_scroll(&mut self) {
        self.scroll_offset = 0;
        self.auto_scroll = false; // Manual reset disables auto-scroll
    }
}
