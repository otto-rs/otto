use std::{
    collections::HashMap,
    sync::Arc,
    time::Duration,
    os::unix::fs::PermissionsExt
};

use eyre::{eyre, Result};
use tokio::{
    process::Command,
    sync::{mpsc, Mutex, Semaphore},
    task::JoinHandle,
    time::timeout,
    io::BufReader,
};
use tracing::{error, info};

use crate::cli::parse::TaskSpec;
use super::{
    workspace::{Workspace, ExecutionContext},
    output::{TaskStreams, OutputType},
};

/// Status of a task during execution
#[derive(Debug, Clone, PartialEq)]
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

/// Classification of task types for optimal execution strategy
#[derive(Debug, Clone)]
pub enum TaskType {
    /// I/O bound tasks like shell commands, file operations
    IOBound,
    /// CPU bound tasks like computation, data processing
    CPUBound,
    /// Network bound tasks like downloads, API calls
    NetworkBound,
}

/// Task scheduler that manages concurrent execution
pub struct TaskScheduler {
    /// Task status tracking
    task_statuses: Arc<Mutex<HashMap<String, TaskStatus>>>,
    /// Semaphore for I/O task limiting
    io_semaphore: Arc<Semaphore>,
    /// Semaphore for CPU task limiting
    cpu_semaphore: Arc<Semaphore>,
    /// Workspace for path management
    workspace: Arc<Workspace>,
    /// Execution context for metadata
    execution_context: ExecutionContext,
    /// Tasks to execute
    tasks: Vec<TaskSpec>,
}

impl TaskScheduler {
    /// Create a new task scheduler
    pub async fn new(
        tasks: Vec<TaskSpec>,
        workspace: Arc<Workspace>,
        execution_context: ExecutionContext,
        io_limit: usize,
        cpu_limit: usize,
    ) -> Result<Self> {
        let task_statuses = Arc::new(Mutex::new(HashMap::new()));

        Ok(Self {
            task_statuses,
            io_semaphore: Arc::new(Semaphore::new(io_limit)),
            cpu_semaphore: Arc::new(Semaphore::new(cpu_limit)),
            workspace,
            execution_context,
            tasks,
        })
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
            TaskType::IOBound => 30,      // 30 seconds
            TaskType::CPUBound => 120,    // 2 minutes
            TaskType::NetworkBound => 60,  // 1 minute
        }
    }

    /// Execute all tasks in the graph
    pub async fn execute_all(&self) -> Result<()> {
        let (tx, mut rx) = mpsc::channel(32);

        // Initialize task statuses
        {
            let mut statuses = self.task_statuses.lock().await;
            for task in &self.tasks {
                statuses.insert(task.name.clone(), TaskStatus::Pending);
            }
        }

        // Initialize in-degree count for all tasks
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut ready_queue = std::collections::VecDeque::new();
        let mut blocked_tasks = Vec::new();

        for task in &self.tasks {
            in_degree.insert(task.name.clone(), task.task_deps.len());

            // Tasks with no dependencies can be queued immediately
            if task.task_deps.is_empty() {
                ready_queue.push_back(task.clone());
            } else {
                blocked_tasks.push(task.clone());
            }
        }

        let mut completed_tasks = 0;
        let total_tasks = self.tasks.len();
        let mut active_tasks = std::collections::HashMap::new();
        let max_concurrent = self.cpu_semaphore.available_permits() + self.io_semaphore.available_permits();

        // Track completed tasks for dependency checking
        let mut completed_set = std::collections::HashSet::new();

        while completed_tasks < total_tasks {
            // Start as many tasks as we can
            while active_tasks.len() < max_concurrent && !ready_queue.is_empty() {
                let task = ready_queue.pop_front().unwrap();

                // Double-check dependencies
                let deps_completed = task.task_deps.iter().all(|dep| completed_set.contains(dep));
                if !deps_completed {
                    // Put it back at the end of the queue
                    ready_queue.push_back(task);

                    // If we can't start any tasks in the queue, wait for completions
                    if ready_queue.len() == 1 {
                        break;
                    }
                    continue;
                }

                info!("Starting task {} ({}/{})", task.name, completed_tasks + 1, total_tasks);

                let handle = self.execute_task(task.clone(), tx.clone()).await?;
                let task_name = task.name.clone();
                active_tasks.insert(task_name.clone(), handle);
            }

            // Wait for any task to complete
            match rx.recv().await {
                Some(Ok(completed_task)) => {
                    info!("Task {} completed successfully", completed_task);
                    let mut statuses = self.task_statuses.lock().await;
                    statuses.insert(completed_task.clone(), TaskStatus::Completed);
                    completed_set.insert(completed_task.clone());
                    completed_tasks += 1;
                    active_tasks.remove(&completed_task);

                    // Check if any blocked tasks are now ready
                    blocked_tasks.retain(|task| {
                        let task_deps_completed = task.task_deps.iter().all(|task_dep| completed_set.contains(task_dep));
                        if !task_deps_completed {
                            return true; // Keep the task in blocked list
                        }

                        // All dependencies are completed, move to ready queue
                        ready_queue.push_back(task.clone());
                        false // Remove from blocked list
                    });

                    // Update in-degree for dependent tasks
                    for remaining_task in &blocked_tasks {
                        if remaining_task.task_deps.contains(&completed_task) {
                            if let Some(degree) = in_degree.get_mut(&remaining_task.name) {
                                *degree = degree.saturating_sub(1);
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    error!("Task execution failed: {}", e);
                    return Err(e);
                }
                None => {
                    error!("Task completion channel closed unexpectedly");
                    return Err(eyre!("Task completion channel closed unexpectedly"));
                }
            }
        }

        Ok(())
    }

    /// Execute a single task
    async fn execute_task(
        &self,
        task: TaskSpec,
        tx: mpsc::Sender<Result<String>>,
    ) -> Result<JoinHandle<Result<()>>> {
        let task_type = Self::classify_task(&task);
        let semaphore = match task_type {
            TaskType::IOBound | TaskType::NetworkBound => self.io_semaphore.clone(),
            TaskType::CPUBound => self.cpu_semaphore.clone(),
        };

        let task_name = task.name.clone();
        let task_dir = self.workspace.task(&task_name);
        let timeout_secs = Self::get_default_timeout(&task_type);
        let task_statuses = self.task_statuses.clone();
        let task_deps = task.task_deps.clone();
        let workspace = self.workspace.clone();
        let script_content = task.action.clone();
        let script_hash = task.hash.clone();
        let envs = task.envs.clone();
        let tasks_dir = self.workspace.run().join("tasks");
        let execution_context = self.execution_context.clone();

        Ok(tokio::spawn(async move {
            // Acquire semaphore permit
            let _permit = semaphore.acquire().await?;

            // Double-check dependencies are still complete before starting
            {
                let statuses = task_statuses.lock().await;
                for dep in &task_deps {
                    if !matches!(statuses.get(dep), Some(TaskStatus::Completed)) {
                        return Err(eyre!("Dependency {} not completed for task {}", dep, task_name));
                    }
                }
            }

            // Update task status to Running ONLY after dependency check
            {
                let mut statuses = task_statuses.lock().await;
                statuses.insert(task_name.clone(), TaskStatus::Running);
            }

            info!("Starting task {}", task_name);

            // Create task directory
            tokio::fs::create_dir_all(&task_dir).await?;

            // Create script file
            let script_path = task_dir.join(format!("{}.sh", script_hash));
            tokio::fs::write(&script_path, &script_content).await?;

            // Make script executable
            let mut perms = tokio::fs::metadata(&script_path).await?.permissions();
            perms.set_mode(0o755);
            tokio::fs::set_permissions(&script_path, perms).await?;

            // Setup command environment
            let mut cmd = Command::new("bash");
            cmd.arg(&script_path)
               .current_dir(&task_dir)
               .env_clear()
               .envs(&envs)
               .env("OTTO_TASK", &task_name)
               .env("OTTO_TASK_DIR", task_dir.to_string_lossy().to_string())
               .env("OTTO_WORKSPACE", workspace.root().to_string_lossy().to_string())
               .env("OTTO_TASKS_DIR", tasks_dir.to_string_lossy().to_string())
               .env("OTTO_USER", &execution_context.user);

            // Execute with timeout
            let result = timeout(Duration::from_secs(timeout_secs), async {
                let mut child = cmd
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()?;

                // Setup output streams
                let stdout = child.stdout.take().ok_or_else(|| eyre!("Failed to capture stdout"))?;
                let stderr = child.stderr.take().ok_or_else(|| eyre!("Failed to capture stderr"))?;

                let streams = TaskStreams::new(&task_name, &tasks_dir).await?;

                // Start output handling
                let stdout_handle = {
                    let streams = streams.clone();
                    let task_name = task_name.clone();
                    tokio::spawn(async move {
                        let reader = BufReader::new(stdout);
                        streams.process_output(task_name, OutputType::Stdout, reader).await
                    })
                };

                let stderr_handle = {
                    let streams = streams.clone();
                    let task_name = task_name.clone();
                    tokio::spawn(async move {
                        let reader = BufReader::new(stderr);
                        streams.process_output(task_name, OutputType::Stderr, reader).await
                    })
                };

                // Wait for process to complete
                let status = child.wait().await?;

                // Wait for output handling to complete
                stdout_handle.await??;
                stderr_handle.await??;

                if status.success() {
                    Ok(())
                } else {
                    Err(eyre!("Task {} failed with exit code {:?}", task_name, status.code()))
                }
            }).await;

            match result {
                Ok(Ok(())) => {
                    info!("Task {} completed successfully", task_name);
                    if tx.send(Ok(task_name.clone())).await.is_err() {
                        error!("Failed to send completion notification for task {}", task_name);
                    }
                }
                Ok(Err(e)) => {
                    error!("Task {} failed: {}", task_name, e);
                    let mut statuses = task_statuses.lock().await;
                    statuses.insert(task_name.clone(), TaskStatus::Failed(e.to_string()));
                    if tx.send(Err(e)).await.is_err() {
                        error!("Failed to send error notification for task {}", task_name);
                    }
                }
                Err(_) => {
                    error!("Task {} timed out after {} seconds", task_name, timeout_secs);
                    let err = eyre!("Task {} timed out after {} seconds", task_name, timeout_secs);
                    let mut statuses = task_statuses.lock().await;
                    statuses.insert(task_name.clone(), TaskStatus::Failed(err.to_string()));
                    if tx.send(Err(err)).await.is_err() {
                        error!("Failed to send timeout notification for task {}", task_name);
                    }
                }
            }

            Ok(())
        }))
    }

    /// Get all task statuses
    pub async fn get_task_statuses(&self) -> HashMap<String, TaskStatus> {
        self.task_statuses.lock().await.clone()
    }

    /// Get the status of a specific task
    pub async fn get_task_status(&self, task_name: &str) -> TaskStatus {
        self.task_statuses.lock().await
            .get(task_name)
            .cloned()
            .unwrap_or(TaskStatus::Pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_task_execution() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());

        let task = TaskSpec::new(
            "test".to_string(),
            vec![],
            HashMap::new(),
            HashMap::new(),
            "echo hello".to_string(),
        );

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(vec![task], Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;
        scheduler.execute_all().await?;

        let status = scheduler.get_task_status("test").await;
        assert_eq!(status, TaskStatus::Completed);

        Ok(())
    }

    #[tokio::test]
    async fn test_task_dependencies() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());

        let tasks = vec![
            TaskSpec::new(
                "task1".to_string(),
                vec!["task2".to_string()],
                HashMap::new(),
                HashMap::new(),
                "echo task1".to_string(),
            ),
            TaskSpec::new(
                "task2".to_string(),
                vec![],
                HashMap::new(),
                HashMap::new(),
                "echo task2".to_string(),
            ),
        ];

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;
        scheduler.execute_all().await?;

        let task1_status = scheduler.get_task_status("task1").await;
        let task2_status = scheduler.get_task_status("task2").await;

        assert_eq!(task1_status, TaskStatus::Completed);
        assert_eq!(task2_status, TaskStatus::Completed);

        Ok(())
    }

    #[tokio::test]
    async fn test_task_failure() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());

        let tasks = vec![
            TaskSpec::new(
                "task1".to_string(),
                vec![],
                HashMap::new(),
                HashMap::new(),
                "exit 1".to_string(),
            ),
            TaskSpec::new(
                "task2".to_string(),
                vec!["task1".to_string()],
                HashMap::new(),
                HashMap::new(),
                "echo task2".to_string(),
            ),
        ];

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;
        let result = scheduler.execute_all().await;

        assert!(result.is_err());

        Ok(())
    }
}
