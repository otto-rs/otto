pub mod action;
pub mod scheduler;
pub mod workspace;
pub mod output;
pub mod visualizer;
pub mod graph;
pub mod colors;

pub use action::{ActionProcessor, ProcessedAction, ScriptProcessor, BashProcessor, PythonProcessor};
pub use colors::{get_task_color, get_task_color_combination, colorize_task_name, colorize_task_prefix, set_global_task_order};
pub use output::TaskStreams;
pub use scheduler::{TaskScheduler, TaskStatus, TaskType};
pub use visualizer::OutputVisualizer;
pub use workspace::Workspace;

// Re-export Task from cli::parse for compatibility
pub use crate::cli::parse::Task;
