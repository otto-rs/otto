use std::{
    path::PathBuf,
    time::SystemTime,
};

use eyre::Result;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::broadcast,
};

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

/// Manages output streams for a task
#[derive(Debug)]
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
    pub async fn new(_task_name: &str, output_dir: &PathBuf) -> Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();

        let stdout_file = output_dir.join(format!("stdout.{}.log", timestamp));
        let stderr_file = output_dir.join(format!("stderr.{}.log", timestamp));

        // Create output directory if it doesn't exist
        if !output_dir.exists() {
            tokio::fs::create_dir_all(output_dir).await?;
        }

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

    /// Process an output stream and write to file
    pub async fn process_output(
        &self,
        task_name: String,
        output_type: OutputType,
        mut reader: impl AsyncBufReadExt + Unpin,
    ) -> Result<()> {
        let output_file = match output_type {
            OutputType::Stdout => &self.stdout_file,
            OutputType::Stderr => &self.stderr_file,
        };

        let mut file = File::create(output_file).await?;
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

            // Write to file
            file.write_all(line.as_bytes()).await?;
            
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
        streams.process_output(
            "test_task".to_string(),
            OutputType::Stdout,
            &mut cursor
        ).await.unwrap();

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
        streams.process_output(
            "test_task".to_string(),
            OutputType::Stdout,
            &mut stdout_cursor
        ).await.unwrap();
        
        streams.process_output(
            "test_task".to_string(),
            OutputType::Stderr,
            &mut stderr_cursor
        ).await.unwrap();
        
        // Verify separate files
        let stdout_contents = streams.read_output(OutputType::Stdout).await.unwrap();
        let stderr_contents = streams.read_output(OutputType::Stderr).await.unwrap();
        
        assert_eq!(stdout_contents[0], "stdout line");
        assert_eq!(stderr_contents[0], "stderr line");
    }
} 