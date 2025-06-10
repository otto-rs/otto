pub mod action;
pub mod scheduler;
pub mod workspace;
pub mod output;
pub mod visualizer;
pub mod graph;

pub use action::{ActionProcessor, ProcessedAction, ScriptProcessor, BashProcessor, PythonProcessor};
pub use output::TaskStreams;
pub use scheduler::{TaskScheduler, TaskStatus, TaskType};
pub use visualizer::OutputVisualizer;
pub use workspace::Workspace;

// Re-export Task from cli::parse for compatibility
pub use crate::cli::parse::Task;
