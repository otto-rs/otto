use super::layout::PaneLayout;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Terminal,
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};
use std::io;
use std::time::{Duration, Instant};

const TUI_TICK_RATE_MS: u64 = 100; // 10 FPS

/// Main TUI application
pub struct TuiApp {
    layout: PaneLayout,
    should_quit: bool,
    last_tick: Instant,
    tick_rate: Duration,
    fullscreen_mode: bool,
    shutdown_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Default for TuiApp {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiApp {
    pub fn new() -> Self {
        Self {
            layout: PaneLayout::new(),
            should_quit: false,
            last_tick: Instant::now(),
            tick_rate: Duration::from_millis(TUI_TICK_RATE_MS),
            fullscreen_mode: false,
            shutdown_flag: None,
        }
    }

    pub fn set_shutdown_flag(&mut self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        self.shutdown_flag = Some(flag);
    }

    pub fn layout_mut(&mut self) -> &mut PaneLayout {
        &mut self.layout
    }

    /// Run the TUI event loop
    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> io::Result<()> {
        loop {
            // Draw UI
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(1),    // Main content area
                        Constraint::Length(3), // Status bar (3 lines with border)
                    ])
                    .split(f.area());

                // Render main content
                if self.fullscreen_mode {
                    self.layout.render_fullscreen(f, chunks[0]);
                } else {
                    self.layout.render(f, chunks[0]);
                }

                // Render status bar
                self.render_status_bar(f, chunks[1]);
            })?;

            // Handle events with timeout
            let timeout = self
                .tick_rate
                .checked_sub(self.last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if crossterm::event::poll(timeout)?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.handle_key_event(key.code);
            }

            // Update tick
            if self.last_tick.elapsed() >= self.tick_rate {
                self.on_tick();
                self.last_tick = Instant::now();
            }

            // Check for Ctrl+C signal
            if let Some(ref flag) = self.shutdown_flag
                && flag.load(std::sync::atomic::Ordering::SeqCst)
            {
                self.should_quit = true;
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    fn on_tick(&mut self) {
        // Update all panes (receive from broadcast channels)
        self.layout.update_all();
    }

    fn render_status_bar(&self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        let total_pages = self.layout.total_pages();
        let current_page = self.layout.current_page() + 1; // 1-indexed for display

        let page_info = if total_pages > 1 {
            format!(" [Page {}/{}] ", current_page, total_pages)
        } else {
            String::new()
        };

        let help_text = if self.fullscreen_mode {
            format!(
                "{}f/Enter: Exit Fullscreen | ↑↓/jk: Scroll | Home: Top | q/Esc: Quit",
                page_info
            )
        } else if total_pages > 1 {
            format!(
                "{}PgUp/PgDn: Change Page | f/Enter: Fullscreen | Tab/←→: Switch | ↑↓/jk: Scroll | q/Esc: Quit",
                page_info
            )
        } else {
            format!(
                "{}f/Enter: Fullscreen | Tab/←→: Switch Pane | ↑↓/jk: Scroll | Home: Top | q/Esc: Quit",
                page_info
            )
        };

        let status_line = Line::from(help_text);
        let paragraph = Paragraph::new(status_line).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );

        frame.render_widget(paragraph, area);
    }

    fn handle_key_event(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Char('f') | KeyCode::Enter => {
                self.fullscreen_mode = !self.fullscreen_mode;
            }
            KeyCode::Tab | KeyCode::Right => {
                self.layout.focus_next();
            }
            KeyCode::BackTab | KeyCode::Left => {
                self.layout.focus_prev();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(pane) = self.layout.focused_pane_mut() {
                    pane.scroll_up();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(pane) = self.layout.focused_pane_mut() {
                    // TODO: Get visible height from render area
                    pane.scroll_down(20);
                }
            }
            KeyCode::Home => {
                if let Some(pane) = self.layout.focused_pane_mut() {
                    pane.reset_scroll();
                }
            }
            KeyCode::PageDown => {
                self.layout.next_page();
            }
            KeyCode::PageUp => {
                self.layout.prev_page();
            }
            _ => {}
        }
    }
}
