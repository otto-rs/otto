use super::layout::PaneLayout;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{Terminal, backend::Backend};
use std::io;
use std::time::{Duration, Instant};

const TUI_TICK_RATE_MS: u64 = 100; // 10 FPS

/// Main TUI application
pub struct TuiApp {
    layout: PaneLayout,
    should_quit: bool,
    last_tick: Instant,
    tick_rate: Duration,
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
        }
    }

    pub fn layout_mut(&mut self) -> &mut PaneLayout {
        &mut self.layout
    }

    /// Run the TUI event loop
    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> io::Result<()> {
        loop {
            // Draw UI
            terminal.draw(|f| self.layout.render(f, f.area()))?;

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

    fn handle_key_event(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
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
            _ => {}
        }
    }
}
