use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use eyre::Result;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::broadcast,
};

use super::{
    clocks::TaskClock,
    colors::{stderr_takes_color, stream_task_label},
    progress::{Stream, facade},
};

/// Capacity of a task's output broadcast channel. Large because a chatty task can
/// emit thousands of lines before a slow subscriber (the TUI) drains them, and a
/// lagged broadcast subscriber loses messages rather than blocking the producer.
const TASK_OUTPUT_CHANNEL_CAPACITY: usize = 10_000;

/// Type of output stream
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Whether to omit the `[task]` prefix from terminal output (`--no-prefix`,
    /// see docs/design/2026-08-28-boundary-fixes-and-dynamic-foreach.md Phase 8).
    /// File output is never prefixed either way, so this only changes what
    /// reaches the terminal.
    no_prefix: bool,
    /// This task's idle clock, shared with its other stream: a line on either
    /// one means the task is not silent.
    clock: Arc<TaskClock>,
}

/// Build the bytes that go to the terminal for one chunk of task output:
/// the `[task]` prefix ahead of the data, or the data alone when `no_prefix`
/// (`--no-prefix`) is set. Pure so the prefix-suppression logic is
/// unit-testable without capturing real process stdout/stderr - which is why
/// `takes_color` comes in as a parameter instead of being read here: this
/// chunk may be headed for stdout or for stderr, and only the caller knows
/// which.
pub(crate) fn format_terminal_output(task_name: &str, data: &[u8], no_prefix: bool, takes_color: bool) -> String {
    if no_prefix {
        String::from_utf8_lossy(data).to_string()
    } else {
        // `no_prefix: false` in the label call is not a contradiction: this arm
        // IS the prefixed case, and that flag chooses `[task]` over a bare
        // `task` inside the label.
        let prefix = stream_task_label(task_name, false, takes_color);
        format!("{} {}", prefix, String::from_utf8_lossy(data))
    }
}

impl TeeWriter {
    pub async fn new(
        file: File,
        is_stderr: bool,
        task_name: String,
        suppress_terminal: bool,
        no_prefix: bool,
        clock: Arc<TaskClock>,
    ) -> Self {
        Self {
            file,
            is_stderr,
            task_name,
            suppress_terminal,
            no_prefix,
            clock,
        }
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        // The task produced a LINE, which is the question the heartbeat asks -
        // not whether bytes reached the terminal. Above the `suppress_terminal`
        // branch below and beside the unconditional file write for that reason:
        // a buffered foreach subtask reaches the terminal only through ordered
        // replay, and a clock keyed on terminal writes would call a chattering
        // subtask idle and heartbeat it continuously. The drain reads
        // `read_until(b'\n')`, so this runs once per newline-terminated line -
        // which is also why a `\r`-redrawn progress bar never gets here, never
        // resets the clock, and is correctly reported as still running.
        self.clock.note_line();

        // Always write to file (no colors)
        self.file.write_all(data).await?;

        // Conditionally write to terminal (suppressed in TUI mode, and for a
        // buffered foreach subtask, whose bytes reach the terminal only through
        // ordered replay)
        if !self.suppress_terminal {
            // Decided per stream, not once for the process: `colored` derives
            // its answer from stdout, and half of what this writer prints goes
            // to stderr. Asking stdout on behalf of stderr is what wrote colour
            // into `otto task 2>log`.
            let takes_color = !self.is_stderr || stderr_takes_color();
            // Write to terminal, with or without the colored task name prefix
            let terminal_output = format_terminal_output(&self.task_name, data, self.no_prefix, takes_color);
            // One facade write per line, never across an `.await`: this is one
            // of the sites a replayed block must not be split by. Per write
            // here, for one whole block in the replay path. The facade flushes,
            // which is what keeps a chatty task's lines in order against the
            // scheduler's own status lines.
            let stream = if self.is_stderr { Stream::Stderr } else { Stream::Stdout };
            facade().write(stream, &terminal_output);
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
    pub async fn new(task_name: &str, output_dir: &Path) -> Result<Self> {
        let task_dir = output_dir.join(task_name);
        if !task_dir.exists() {
            tokio::fs::create_dir_all(&task_dir).await?;
        }

        let stdout_file = task_dir.join("stdout.log");
        let stderr_file = task_dir.join("stderr.log");

        // Not created here: `process_output` creates (and truncates) whichever
        // of these it is handed the moment it actually starts writing, so an
        // earlier create here was pure redundant I/O with nothing reading the
        // file in between.
        let (output_tx, _) = broadcast::channel(TASK_OUTPUT_CHANNEL_CAPACITY);

        Ok(Self {
            stdout_file,
            stderr_file,
            output_tx,
        })
    }

    pub async fn process_output(
        &self,
        task_name: String,
        output_type: OutputType,
        mut reader: impl AsyncBufReadExt + Unpin,
        suppress_terminal: bool,
        no_prefix: bool,
        clock: Arc<TaskClock>,
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
            no_prefix,
            clock,
        )
        .await;

        // Drain bytes, not `String`s. `read_line` decodes UTF-8 and returns
        // InvalidData on the first bad byte; the old `while let Ok(n)` read that
        // error as end-of-stream, so one stray byte truncated the terminal output
        // AND the log file at that line and the task still reported success. Bytes
        // cannot fail to decode: display goes through `from_utf8_lossy`, exactly
        // like `TeeWriter::write` already does, and the log keeps the raw bytes.
        let mut line: Vec<u8> = Vec::new();

        loop {
            line.clear();
            let n = match reader.read_until(b'\n', &mut line).await {
                Ok(n) => n,
                Err(e) => {
                    // A real read error ends the drain, and says so rather than
                    // passing for a clean EOF.
                    log::warn!("Error reading {output_type:?} for task '{task_name}': {e}");
                    break;
                }
            };
            if n == 0 {
                break;
            }

            let output = TaskOutput {
                task_name: task_name.clone(),
                stream_type: output_type,
                timestamp: SystemTime::now(),
                content: String::from_utf8_lossy(&line).into_owned(),
            };

            // Write to both file and terminal
            writer.write(&line).await?;

            // Broadcast for real-time monitoring
            let _ = self.output_tx.send(output);
        }

        Ok(())
    }

    pub async fn read_output(&self, output_type: OutputType) -> Result<Vec<String>> {
        let file_path = match output_type {
            OutputType::Stdout => &self.stdout_file,
            OutputType::Stderr => &self.stderr_file,
        };

        // Read back the same way the log is written: bytes, decoded lossily.
        // `lines()` errors on the first non-UTF-8 byte, which would make a log
        // that faithfully captured a task's binary output unreadable to otto.
        let mut lines = Vec::new();
        let file = File::open(file_path).await?;
        let mut reader = BufReader::new(file);
        let mut buffer: Vec<u8> = Vec::new();

        loop {
            buffer.clear();
            if reader.read_until(b'\n', &mut buffer).await? == 0 {
                break;
            }
            while matches!(buffer.last(), Some(b'\n' | b'\r')) {
                buffer.pop();
            }
            lines.push(String::from_utf8_lossy(&buffer).into_owned());
        }

        Ok(lines)
    }
}

#[path = "output_tests.rs"]
mod tests;
