use super::pane::Pane;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
};

/// Manages dynamic pane layout
pub struct PaneLayout {
    panes: Vec<Box<dyn Pane>>,
    focused_index: usize,
}

impl PaneLayout {
    pub fn new() -> Self {
        Self {
            panes: Vec::new(),
            focused_index: 0,
        }
    }

    pub fn add_pane(&mut self, pane: Box<dyn Pane>) {
        self.panes.push(pane);
    }

    pub fn update_all(&mut self) {
        for pane in &mut self.panes {
            pane.update();
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if self.panes.is_empty() {
            return;
        }

        let grid_areas = self.calculate_grid(area);

        for (i, pane) in self.panes.iter().enumerate() {
            if let Some(pane_area) = grid_areas.get(i) {
                let focused = i == self.focused_index;
                pane.render(frame, *pane_area, focused);
            }
        }
    }

    pub fn render_fullscreen(&self, frame: &mut Frame, area: Rect) {
        if self.panes.is_empty() {
            return;
        }

        // Render only the focused pane in fullscreen
        if let Some(pane) = self.panes.get(self.focused_index) {
            pane.render(frame, area, true);
        }
    }

    fn calculate_grid(&self, area: Rect) -> Vec<Rect> {
        let num_panes = self.panes.len();

        if num_panes == 0 {
            return vec![];
        }

        // Determine grid dimensions based on pane count
        let (rows, cols) = match num_panes {
            1 => (1, 1),
            2 => (1, 2),
            3..=4 => (2, 2),
            5..=6 => (2, 3),
            7..=9 => (3, 3),
            10..=12 => (3, 4),
            _ => (4, 4), // Max 16 visible panes
        };

        // Create row constraints
        let row_constraints: Vec<Constraint> = (0..rows).map(|_| Constraint::Percentage(100 / rows as u16)).collect();

        let row_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(row_constraints)
            .split(area);

        // Create column constraints for each row
        let col_constraints: Vec<Constraint> = (0..cols).map(|_| Constraint::Percentage(100 / cols as u16)).collect();

        let mut grid_areas = Vec::new();
        for row_area in row_layout.iter() {
            let col_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(&col_constraints)
                .split(*row_area);

            grid_areas.extend(col_layout.iter().copied());
        }

        // Return only as many areas as we have panes
        grid_areas.truncate(num_panes);
        grid_areas
    }

    pub fn focus_next(&mut self) {
        if !self.panes.is_empty() {
            self.focused_index = (self.focused_index + 1) % self.panes.len();
        }
    }

    pub fn focus_prev(&mut self) {
        if !self.panes.is_empty() {
            self.focused_index = if self.focused_index == 0 {
                self.panes.len() - 1
            } else {
                self.focused_index - 1
            };
        }
    }

    pub fn focused_pane_mut(&mut self) -> Option<&mut Box<dyn Pane>> {
        self.panes.get_mut(self.focused_index)
    }
}

impl Default for PaneLayout {
    fn default() -> Self {
        Self::new()
    }
}
