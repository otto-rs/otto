use chrono::{Local, TimeZone};
use colored::Colorize;
use console::measure_text_width;
use eyre::Result;

use crate::executor::{RunStatus, StateManager};

fn display_width(s: &str) -> usize {
    measure_text_width(s)
}

/// Pad string to exact width (left-align)
fn pad_left(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

/// Pad string to exact width (right-align)
fn pad_right(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - w), s)
    }
}

/// Center within a field
fn pad_center(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        let total = width - w;
        let left = total / 2;
        let right = total - left;
        format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
    }
}

/// Show execution history
#[derive(Debug, clap::Parser)]
#[command(name = "history")]
pub struct HistoryCommand {
    /// Show history for a specific task
    #[arg(value_name = "TASK")]
    pub task_name: Option<String>,

    /// Limit number of results
    #[arg(short = 'n', long, default_value = "20")]
    pub limit: usize,

    /// Filter by status (success, failed, running)
    #[arg(short, long)]
    pub status: Option<String>,

    /// Filter by project hash
    #[arg(short, long)]
    pub project: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl HistoryCommand {
    pub fn execute(&self) -> Result<()> {
        let manager = match StateManager::try_new() {
            Some(m) => m,
            None => {
                eprintln!("{}", "No history database found. Run otto to create it.".yellow());
                return Ok(());
            }
        };

        if let Some(ref task_name) = self.task_name {
            self.show_task_history(&manager, task_name)
        } else {
            self.show_run_history(&manager)
        }
    }

    fn show_run_history(&self, manager: &StateManager) -> Result<()> {
        let status_filter = self.status.as_ref().and_then(|s| match s.as_str() {
            "success" => Some(RunStatus::Success),
            "failed" => Some(RunStatus::Failed),
            "running" => Some(RunStatus::Running),
            _ => None,
        });

        let runs = manager.get_runs_with_filters(status_filter, self.project.as_deref(), self.limit)?;

        if runs.is_empty() {
            println!("{}", "No runs found.".yellow());
            return Ok(());
        }

        if self.json {
            println!("{}", serde_json::to_string_pretty(&runs)?);
            return Ok(());
        }

        let mut rows: Vec<(String, String, String, String, String, String)> = Vec::new();

        for run in &runs {
            let path = run
                .cwd
                .as_ref()
                .and_then(|p| p.to_str())
                .map(|s| {
                    if let Ok(home) = std::env::var("HOME")
                        && s.starts_with(&home)
                    {
                        s.replace(&home, "~")
                    } else {
                        s.to_string()
                    }
                })
                .unwrap_or_else(|| "-".to_string());

            rows.push((
                format_timestamp(run.timestamp),
                format_run_status(&run.status),
                format_duration(run.duration_seconds),
                format_size(run.size_bytes),
                run.user.clone().unwrap_or_else(|| "-".to_string()),
                path,
            ));
        }

        // Calculate max width for each column
        let mut w1 = display_width("Timestamp");
        let mut w2 = display_width("Status");
        let mut w3 = display_width("Duration");
        let mut w4 = display_width("Size");
        let mut w5 = display_width("User");
        let mut w6 = display_width("Path");

        for (c1, c2, c3, c4, c5, c6) in &rows {
            w1 = w1.max(display_width(c1));
            w2 = w2.max(display_width(c2));
            w3 = w3.max(display_width(c3));
            w4 = w4.max(display_width(c4));
            w5 = w5.max(display_width(c5));
            w6 = w6.max(display_width(c6));
        }

        // Print header
        println!();
        println!(
            "{}  {}  {}  {}  {}  {}",
            pad_left("Timestamp", w1).bold(),
            pad_center("Status", w2).bold(),
            pad_right("Duration", w3).bold(),
            pad_right("Size", w4).bold(),
            pad_left("User", w5).bold(),
            pad_left("Path", w6).bold(),
        );

        let total_width = w1 + w2 + w3 + w4 + w5 + w6 + 10;
        println!("{}", "─".repeat(total_width).dimmed());

        // Print rows
        for (c1, c2, c3, c4, c5, c6) in &rows {
            println!(
                "{}  {}  {}  {}  {}  {}",
                pad_left(c1, w1),
                pad_center(c2, w2),
                pad_right(c3, w3),
                pad_right(c4, w4),
                pad_left(c5, w5),
                pad_left(c6, w6),
            );
        }

        println!("\nTotal runs: {}", runs.len());
        Ok(())
    }

    fn show_task_history(&self, manager: &StateManager, task_name: &str) -> Result<()> {
        let history = manager.get_task_history(task_name, self.limit)?;

        if history.is_empty() {
            println!("{}", format!("No history found for task '{}'.", task_name).yellow());
            return Ok(());
        }

        if self.json {
            println!("{}", serde_json::to_string_pretty(&history)?);
            return Ok(());
        }

        println!("\n{} for task '{}'", "History".bold(), task_name.cyan());

        let mut rows: Vec<(String, String, String, String, String, String)> = Vec::new();

        for task in &history {
            rows.push((
                task.started_at.map(format_timestamp).unwrap_or_else(|| "-".to_string()),
                format_task_status(&task.status),
                format_duration(task.duration_seconds),
                task.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "-".to_string()),
                if task.interactive { "yes".to_string() } else { "no".to_string() },
                task.run_id.to_string(),
            ));
        }

        let mut w1 = display_width("Timestamp");
        let mut w2 = display_width("Status");
        let mut w3 = display_width("Duration");
        let mut w4 = display_width("Exit Code");
        let mut w5 = display_width("Interactive");
        let mut w6 = display_width("Run ID");

        for (c1, c2, c3, c4, c5, c6) in &rows {
            w1 = w1.max(display_width(c1));
            w2 = w2.max(display_width(c2));
            w3 = w3.max(display_width(c3));
            w4 = w4.max(display_width(c4));
            w5 = w5.max(display_width(c5));
            w6 = w6.max(display_width(c6));
        }

        println!();
        println!(
            "{}  {}  {}  {}  {}  {}",
            pad_left("Timestamp", w1).bold(),
            pad_center("Status", w2).bold(),
            pad_right("Duration", w3).bold(),
            pad_center("Exit Code", w4).bold(),
            pad_center("Interactive", w5).bold(),
            pad_right("Run ID", w6).bold(),
        );

        let total_width = w1 + w2 + w3 + w4 + w5 + w6 + 10;
        println!("{}", "─".repeat(total_width).dimmed());

        for (c1, c2, c3, c4, c5, c6) in &rows {
            println!(
                "{}  {}  {}  {}  {}  {}",
                pad_left(c1, w1),
                pad_center(c2, w2),
                pad_right(c3, w3),
                pad_center(c4, w4),
                pad_center(c5, w5),
                pad_right(c6, w6),
            );
        }

        println!("\nTotal executions: {}", history.len());

        let successful = history
            .iter()
            .filter(|t| matches!(t.status, crate::executor::state::TaskStatus::Completed))
            .count();
        let failed = history
            .iter()
            .filter(|t| matches!(t.status, crate::executor::state::TaskStatus::Failed))
            .count();

        if successful + failed > 0 {
            let success_rate = (successful as f64 / (successful + failed) as f64) * 100.0;
            println!("Success rate: {:.1}%", success_rate);
        }

        Ok(())
    }
}

fn format_timestamp(timestamp: u64) -> String {
    let dt = Local.timestamp_opt(timestamp as i64, 0).unwrap();
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn format_duration(duration: Option<f64>) -> String {
    match duration {
        Some(d) if d < 1.0 => format!("{:.0}ms", d * 1000.0),
        Some(d) if d < 60.0 => format!("{:.1}s", d),
        Some(d) if d < 3600.0 => {
            let minutes = (d / 60.0) as u64;
            let seconds = (d % 60.0) as u64;
            format!("{}m{}s", minutes, seconds)
        }
        Some(d) => {
            let hours = (d / 3600.0) as u64;
            let minutes = ((d % 3600.0) / 60.0) as u64;
            format!("{}h{}m", hours, minutes)
        }
        None => "-".to_string(),
    }
}

fn format_size(size: Option<u64>) -> String {
    match size {
        Some(s) if s < 1024 => format!("{} B", s),
        Some(s) if s < 1024 * 1024 => format!("{:.1} KB", s as f64 / 1024.0),
        Some(s) if s < 1024 * 1024 * 1024 => format!("{:.1} MB", s as f64 / (1024.0 * 1024.0)),
        Some(s) => format!("{:.2} GB", s as f64 / (1024.0 * 1024.0 * 1024.0)),
        None => "-".to_string(),
    }
}

fn format_run_status(status: &RunStatus) -> String {
    match status {
        RunStatus::Success => "✓".green().to_string(),
        RunStatus::Failed => "✗".red().to_string(),
        RunStatus::Running => "⋯".yellow().to_string(),
    }
}

fn format_task_status(status: &crate::executor::state::TaskStatus) -> String {
    use crate::executor::state::TaskStatus;
    match status {
        TaskStatus::Completed => "✓".green().to_string(),
        TaskStatus::Failed => "✗".red().to_string(),
        TaskStatus::Running => "⋯".yellow().to_string(),
        TaskStatus::Skipped => "○".blue().to_string(),
        TaskStatus::Pending => "·".dimmed().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Some(0.5)), "500ms");
        assert_eq!(format_duration(Some(1.5)), "1.5s");
        assert_eq!(format_duration(Some(65.0)), "1m5s");
        assert_eq!(format_duration(Some(3665.0)), "1h1m");
        assert_eq!(format_duration(None), "-");
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(Some(500)), "500 B");
        assert_eq!(format_size(Some(1536)), "1.5 KB");
        assert_eq!(format_size(Some(1572864)), "1.5 MB");
        assert_eq!(format_size(Some(1610612736)), "1.50 GB");
        assert_eq!(format_size(None), "-");
    }

    #[test]
    fn test_format_timestamp() {
        let timestamp = 1234567890;
        let result = format_timestamp(timestamp);
        assert!(result.contains("-"));
        assert!(result.contains(":"));
    }
}
