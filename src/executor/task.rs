use std::{
    collections::HashMap,
    path::PathBuf,
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use hex;

/// Classification of task types for optimal execution strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskType {
    /// I/O bound tasks like shell commands, file operations
    IOBound,
    /// CPU bound tasks like computation, data processing
    CPUBound,
    /// Network bound tasks like downloads, API calls
    NetworkBound,
}

/// Status of a task during execution
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task is waiting for dependencies
    Pending,
    /// Task is currently running
    Running,
    /// Task completed successfully
    Completed,
    /// Task failed during execution
    Failed(String),
}

/// Core task specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Unique task name
    pub name: String,
    /// Command or action to execute
    pub action: String,
    /// Task dependencies
    pub deps: Vec<String>,
    /// Environment variables
    pub envs: HashMap<String, String>,
    /// Task-specific working directory
    pub working_dir: Option<PathBuf>,
    /// Task timeout in seconds (0 = no timeout)
    pub timeout: u64,
}

/// A task ready for execution
#[derive(Debug, Clone)]
pub struct Task {
    /// Task specification
    pub spec: TaskSpec,
    /// Task classification
    pub task_type: TaskType,
    /// Current status
    pub status: TaskStatus,
    /// Creation timestamp
    pub created_at: SystemTime,
}

impl Task {
    /// Create a new task from a specification
    pub fn new(spec: TaskSpec, _work_dir: PathBuf) -> Self {
        let task_type = Self::classify_task(&spec);
        let timeout = if spec.timeout == 0 {
            Self::get_default_timeout(&task_type)
        } else {
            spec.timeout
        };

        Self {
            task_type,
            status: TaskStatus::Pending,
            spec: TaskSpec { timeout, ..spec },
            created_at: SystemTime::now(),
        }
    }

    /// Calculate task hash for storage
    pub fn calculate_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&self.spec.action);
        let result = hasher.finalize();
        hex::encode(&result)[..8].to_string()
    }

    /// Classify task based on its properties
    fn classify_task(spec: &TaskSpec) -> TaskType {
        let cmd = spec.action.to_lowercase();
        
        // Network operations
        if cmd.contains("curl") || cmd.contains("wget") || 
           cmd.contains("http") || cmd.contains("ssh") {
            return TaskType::NetworkBound;
        }
        
        // CPU intensive operations
        if cmd.contains("gcc") || cmd.contains("rustc") || 
           cmd.contains("make") || cmd.contains("cargo build") ||
           cmd.contains("cargo test") || cmd.contains("cargo check") ||
           cmd.contains("cmake") || cmd.contains("ninja") {
            return TaskType::CPUBound;
        }
        
        // Default to I/O bound
        TaskType::IOBound
    }

    /// Get default timeout based on task type
    fn get_default_timeout(task_type: &TaskType) -> u64 {
        match task_type {
            TaskType::IOBound => 60,      // 1 minute
            TaskType::CPUBound => 300,    // 5 minutes
            TaskType::NetworkBound => 180, // 3 minutes
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_task_classification() {
        let cases = vec![
            ("curl https://example.com", TaskType::NetworkBound),
            ("gcc -c file.c", TaskType::CPUBound),
            ("cat file.txt", TaskType::IOBound),
        ];

        for (action, _expected_type) in cases {
            let spec = TaskSpec {
                name: "test".to_string(),
                action: action.to_string(),
                deps: vec![],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 0,
            };
            
            assert!(matches!(
                Task::classify_task(&spec),
                _expected_type
            ));
        }
    }

    #[test]
    fn test_task_hash_stability() {
        let spec = TaskSpec {
            name: "test".to_string(),
            action: "echo hello".to_string(),
            deps: vec!["dep1".to_string()],
            envs: HashMap::new(),
            working_dir: None,
            timeout: 0,
        };

        let task = Task::new(spec.clone(), PathBuf::from("/tmp"));
        let hash1 = task.calculate_hash();
        let task2 = Task::new(spec, PathBuf::from("/tmp"));
        let hash2 = task2.calculate_hash();

        assert_eq!(hash1, hash2, "Same task should produce same hash");
    }
} 