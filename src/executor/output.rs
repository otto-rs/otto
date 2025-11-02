use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

use eyre::Result;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::broadcast,
};

use super::colors::colorize_task_prefix;

/// Type of output stream
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OutputType {
    /// Standard output
    Stdout,
    /// Standard error
    Stderr,
}

/// A single line of task output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutput {
    /// Name of the task that produced this output
    pub task_name: String,
    /// Type of output stream
    pub stream_type: OutputType,
    /// When this output was produced
    pub timestamp: SystemTime,
    /// The actual output content
    pub content: String,
}

/// Status of a task for TUI display
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TuiTaskStatus {
    /// Task is waiting to be executed
    Pending,
    /// Task is currently running
    Running,
    /// Task completed successfully
    Completed,
    /// Task failed with an error
    Failed,
    /// Task was skipped
    Skipped,
}

/// Message sent from scheduler to TUI with task updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskMessage {
    /// Task output line
    Output(TaskOutput),
    /// Task status change
    StatusChange {
        task_name: String,
        status: TuiTaskStatus,
        timestamp: SystemTime,
    },
    /// Task started executing
    Started { task_name: String, timestamp: SystemTime },
    /// Task finished executing
    Finished {
        task_name: String,
        status: TuiTaskStatus,
        timestamp: SystemTime,
        duration_ms: u64,
    },
}

/// A writer that writes to both a file and a terminal
pub struct TeeWriter {
    /// File to write to
    file: File,
    /// Whether this is stderr (true) or stdout (false)
    is_stderr: bool,
    /// Task name for prefixing output
    task_name: String,
    /// Whether to suppress terminal output (for TUI mode)
    suppress_terminal: bool,
}

impl TeeWriter {
    /// Create a new TeeWriter
    pub async fn new(file: File, is_stderr: bool, task_name: String, suppress_terminal: bool) -> Self {
        Self {
            file,
            is_stderr,
            task_name,
            suppress_terminal,
        }
    }

    /// Write data to both file and terminal
    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        // Always write to file (no colors)
        self.file.write_all(data).await?;

        // Conditionally write to terminal (suppressed in TUI mode)
        if !self.suppress_terminal {
            // Write to terminal with colored task name prefix
            let colored_prefix = colorize_task_prefix(&self.task_name);
            let terminal_output = format!("{} {}", colored_prefix, String::from_utf8_lossy(data));
            if self.is_stderr {
                eprint!("{terminal_output}");
            } else {
                print!("{terminal_output}");
            }

            // Ensure terminal output is flushed
            if self.is_stderr {
                io::stderr().flush()?;
            } else {
                io::stdout().flush()?;
            }
        }

        Ok(())
    }
}

/// Manages output streams for a task
#[derive(Debug, Clone)]
pub struct TaskStreams {
    /// Path to stdout log file
    pub stdout_file: PathBuf,
    /// Path to stderr log file
    pub stderr_file: PathBuf,
    /// Broadcast channel for real-time output
    pub output_tx: broadcast::Sender<TaskOutput>,
}

impl TaskStreams {
    /// Create new output streams for a task
    pub async fn new(task_name: &str, output_dir: &Path) -> Result<Self> {
        // Create task directory if it doesn't exist
        let task_dir = output_dir.join(task_name);
        if !task_dir.exists() {
            tokio::fs::create_dir_all(&task_dir).await?;
        }

        let stdout_file = task_dir.join("stdout.log");
        let stderr_file = task_dir.join("stderr.log");

        // Create empty log files
        File::create(&stdout_file).await?;
        File::create(&stderr_file).await?;

        let (output_tx, _) = broadcast::channel(100);

        Ok(Self {
            stdout_file,
            stderr_file,
            output_tx,
        })
    }

    /// Process an output stream and write to file and terminal
    pub async fn process_output(
        &self,
        task_name: String,
        output_type: OutputType,
        mut reader: impl AsyncBufReadExt + Unpin,
        suppress_terminal: bool,
    ) -> Result<()> {
        let output_file = match output_type {
            OutputType::Stdout => &self.stdout_file,
            OutputType::Stderr => &self.stderr_file,
        };

        let file = File::create(output_file).await?;
        let mut writer = TeeWriter::new(
            file,
            matches!(output_type, OutputType::Stderr),
            task_name.clone(),
            suppress_terminal,
        )
        .await;

        let mut line = String::new();

        while let Ok(n) = reader.read_line(&mut line).await {
            if n == 0 {
                break;
            }

            let output = TaskOutput {
                task_name: task_name.clone(),
                stream_type: output_type.clone(),
                timestamp: SystemTime::now(),
                content: line.clone(),
            };

            // Write to both file and terminal
            writer.write(line.as_bytes()).await?;

            // Broadcast for real-time monitoring
            let _ = self.output_tx.send(output);

            line.clear();
        }

        Ok(())
    }

    /// Read all output from a specific stream
    pub async fn read_output(&self, output_type: OutputType) -> Result<Vec<String>> {
        let file_path = match output_type {
            OutputType::Stdout => &self.stdout_file,
            OutputType::Stderr => &self.stderr_file,
        };

        let mut lines = Vec::new();
        let file = File::open(file_path).await?;
        let mut reader = BufReader::new(file).lines();

        while let Some(line) = reader.next_line().await? {
            lines.push(line);
        }

        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_output_processing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_dir = PathBuf::from(temp_dir.path());

        let streams = TaskStreams::new("test_task", &output_dir).await.unwrap();

        // Create a test reader with some output
        let test_output = "line 1\nline 2\nline 3\n";
        let mut rx = streams.output_tx.subscribe();

        // Process the output
        let mut cursor = std::io::Cursor::new(test_output);
        streams
            .process_output("test_task".to_string(), OutputType::Stdout, &mut cursor, false)
            .await
            .unwrap();

        // Verify file contents
        let contents = streams.read_output(OutputType::Stdout).await.unwrap();
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[0], "line 1");

        // Verify broadcast channel
        let received = rx.try_recv().unwrap();
        assert_eq!(received.task_name, "test_task");
        assert_eq!(received.content, "line 1\n");
    }

    #[tokio::test]
    async fn test_multiple_streams() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_dir = PathBuf::from(temp_dir.path());

        let streams = TaskStreams::new("test_task", &output_dir).await.unwrap();

        // Write to both stdout and stderr
        let stdout_data = "stdout line\n";
        let stderr_data = "stderr line\n";

        let mut stdout_cursor = std::io::Cursor::new(stdout_data);
        let mut stderr_cursor = std::io::Cursor::new(stderr_data);

        // Process both streams
        streams
            .process_output("test_task".to_string(), OutputType::Stdout, &mut stdout_cursor, false)
            .await
            .unwrap();

        streams
            .process_output("test_task".to_string(), OutputType::Stderr, &mut stderr_cursor, false)
            .await
            .unwrap();

        // Verify separate files
        let stdout_contents = streams.read_output(OutputType::Stdout).await.unwrap();
        let stderr_contents = streams.read_output(OutputType::Stderr).await.unwrap();

        assert_eq!(stdout_contents[0], "stdout line");
        assert_eq!(stderr_contents[0], "stderr line");
    }
}
