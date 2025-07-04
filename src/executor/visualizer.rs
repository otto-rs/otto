use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::sync::{broadcast, Mutex};
use log::info;
use eyre::Result;

use super::output::{TaskOutput, OutputType};

/// Configuration for output visualization
#[derive(Debug, Clone)]
pub struct VisualizerConfig {
    /// Maximum number of lines to keep in memory per stream
    pub max_lines: usize,
    /// Whether to show timestamps
    pub show_timestamps: bool,
    /// Whether to show task names
    pub show_task_names: bool,
    /// Whether to show stream types
    pub show_stream_types: bool,
}

impl Default for VisualizerConfig {
    fn default() -> Self {
        Self {
            max_lines: 1000,
            show_timestamps: true,
            show_task_names: true,
            show_stream_types: true,
        }
    }
}

/// Manages visualization of multiple task outputs
#[derive(Debug)]
pub struct OutputVisualizer {
    /// Output receiver for all tasks
    output_rx: broadcast::Receiver<TaskOutput>,
    /// Buffer of recent output lines per task
    output_buffers: Arc<Mutex<HashMap<String, Vec<TaskOutput>>>>,
    /// Visualization configuration
    config: VisualizerConfig,
}

impl OutputVisualizer {
    /// Create a new output visualizer
    pub fn new(output_rx: broadcast::Receiver<TaskOutput>, config: VisualizerConfig) -> Self {
        Self {
            output_rx,
            output_buffers: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    /// Start the visualization loop
    pub async fn run(&mut self) -> Result<()> {
        info!("Starting output visualizer");
        
        while let Ok(output) = self.output_rx.recv().await {
            self.process_output(output).await?;
        }

        Ok(())
    }

    /// Process a new output line
    async fn process_output(&self, output: TaskOutput) -> Result<()> {
        let mut buffers = self.output_buffers.lock().await;
        
        let buffer = buffers
            .entry(output.task_name.clone())
            .or_insert_with(Vec::new);

        // Add new output
        buffer.push(output.clone());

        // Trim buffer if needed
        if buffer.len() > self.config.max_lines {
            buffer.remove(0);
        }

        // Format and display the output
        let formatted = self.format_output(&output);
        println!("{}", formatted);

        Ok(())
    }

    /// Format an output line according to configuration
    fn format_output(&self, output: &TaskOutput) -> String {
        let mut parts = Vec::new();

        if self.config.show_timestamps {
            let timestamp = output.timestamp
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or(Duration::from_secs(0))
                .as_secs();
            parts.push(format!("[{}]", timestamp));
        }

        if self.config.show_task_names {
            parts.push(format!("[{}]", output.task_name));
        }

        if self.config.show_stream_types {
            let stream = match output.stream_type {
                OutputType::Stdout => "out",
                OutputType::Stderr => "err",
            };
            parts.push(format!("[{}]", stream));
        }

        parts.push(output.content.clone());
        parts.join(" ")
    }

    /// Get recent output for a specific task
    pub async fn get_task_output(&self, task_name: &str) -> Vec<TaskOutput> {
        let buffers = self.output_buffers.lock().await;
        buffers.get(task_name)
            .cloned()
            .unwrap_or_default()
    }

    /// Clear output buffer for a task
    pub async fn clear_task_output(&self, task_name: &str) {
        let mut buffers = self.output_buffers.lock().await;
        if let Some(buffer) = buffers.get_mut(task_name) {
            buffer.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    #[tokio::test]
    async fn test_output_formatting() {
        let (_tx, rx) = broadcast::channel(100);
        let config = VisualizerConfig {
            show_timestamps: true,
            show_task_names: true,
            show_stream_types: true,
            max_lines: 10,
        };

        let visualizer = OutputVisualizer::new(rx, config);

        let output = TaskOutput {
            task_name: "test_task".to_string(),
            stream_type: OutputType::Stdout,
            timestamp: SystemTime::UNIX_EPOCH,
            content: "test output".to_string(),
        };

        let formatted = visualizer.format_output(&output);
        assert!(formatted.contains("[0]")); // timestamp
        assert!(formatted.contains("[test_task]")); // task name
        assert!(formatted.contains("[out]")); // stream type
        assert!(formatted.contains("test output")); // content
    }

    #[tokio::test]
    async fn test_buffer_management() {
        let (_tx, rx) = broadcast::channel(100);
        let config = VisualizerConfig {
            max_lines: 2,
            ..Default::default()
        };

        let visualizer = OutputVisualizer::new(rx, config);

        // Add three outputs (should only keep latest two)
        for i in 1..=3 {
            let output = TaskOutput {
                task_name: "test_task".to_string(),
                stream_type: OutputType::Stdout,
                timestamp: SystemTime::UNIX_EPOCH,
                content: format!("output {}", i),
            };
            visualizer.process_output(output).await.unwrap();
        }

        let outputs = visualizer.get_task_output("test_task").await;
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].content, "output 2");
        assert_eq!(outputs[1].content, "output 3");
    }
} 