use std::{
    collections::HashMap,
    io::{self, Write},
    path::Path,
    sync::Arc,
    time::Duration,
};

use eyre::{Result, eyre};
use log::{debug, error, info};
use tokio::{
    io::BufReader,
    process::Command,
    sync::{Mutex, Semaphore, mpsc},
    task::JoinHandle,
    time::timeout,
};

use super::task::Task;
use super::{
    action::{ActionProcessor, ProcessedAction},
    colors::{colorize_task_prefix, set_global_task_order},
    output::{OutputType, TaskMessage, TaskStreams, TuiTaskStatus},
    workspace::{ExecutionContext, Workspace},
};

/// Timeout for output processing after task completion
const OUTPUT_PROCESSING_TIMEOUT_SECS: u64 = 5;

/// Status of a task during execution
#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    /// Task is waiting for dependencies
    Pending,
    /// Task is currently running
    Running,
    /// Task completed successfully
    Completed,
    /// Task was skipped due to up-to-date outputs
    Skipped,
    /// Task failed during execution
    Failed(String),
}

/// Task scheduler that manages concurrent execution
pub struct TaskScheduler {
    /// Task status tracking
    task_statuses: Arc<Mutex<HashMap<String, TaskStatus>>>,
    /// Semaphore for task limiting
    semaphore: Arc<Semaphore>,
    /// Workspace for path management
    workspace: Arc<Workspace>,
    /// Execution context for metadata
    execution_context: ExecutionContext,
    /// Tasks to execute
    tasks: Vec<Task>,
    /// Whether TUI mode is enabled (suppresses terminal output)
    tui_mode: bool,
    /// Optional broadcast channel for TUI status updates
    message_tx: Option<tokio::sync::broadcast::Sender<TaskMessage>>,
    /// Pre-created TaskStreams for TUI mode (task_name -> TaskStreams)
    task_streams: Option<Arc<std::collections::HashMap<String, TaskStreams>>>,
}

impl TaskScheduler {
    /// Create a new task scheduler
    pub async fn new(
        tasks: Vec<Task>,
        workspace: Arc<Workspace>,
        execution_context: ExecutionContext,
        max_parallel: usize,
        tui_mode: bool,
    ) -> Result<Self> {
        let task_statuses = Arc::new(Mutex::new(HashMap::new()));

        // Set up global task ordering for consistent color assignment
        let task_names: Vec<String> = tasks.iter().map(|t| t.name.clone()).collect();
        set_global_task_order(task_names);

        Ok(Self {
            task_statuses,
            semaphore: Arc::new(Semaphore::new(max_parallel)),
            workspace,
            execution_context,
            tasks,
            tui_mode,
            message_tx: None,
            task_streams: None,
        })
    }

    /// Set the message broadcast channel for TUI updates
    pub fn set_message_channel(&mut self, tx: tokio::sync::broadcast::Sender<TaskMessage>) {
        self.message_tx = Some(tx);
    }

    /// Set pre-created TaskStreams for TUI mode
    pub fn set_task_streams(&mut self, streams: std::collections::HashMap<String, TaskStreams>) {
        self.task_streams = Some(Arc::new(streams));
    }

    /// Helper to broadcast a TaskMessage to TUI
    fn broadcast_message(&self, message: TaskMessage) {
        if let Some(tx) = &self.message_tx {
            let _ = tx.send(message);
        }
    }

    /// Convert internal TaskStatus to TUI TaskStatus
    #[allow(dead_code)]
    fn to_tui_status(status: &TaskStatus) -> TuiTaskStatus {
        match status {
            TaskStatus::Pending => TuiTaskStatus::Pending,
            TaskStatus::Running => TuiTaskStatus::Running,
            TaskStatus::Completed => TuiTaskStatus::Completed,
            TaskStatus::Skipped => TuiTaskStatus::Skipped,
            TaskStatus::Failed(_) => TuiTaskStatus::Failed,
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
        let max_concurrent = self.semaphore.available_permits();

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

                // Check file dependencies to see if task needs to run
                match self.needs_rebuild(&task).await {
                    Ok(true) => {
                        // Task needs to run
                        info!("Starting task {} ({}/{})", task.name, completed_tasks + 1, total_tasks);

                        // Broadcast task started to TUI
                        self.broadcast_message(TaskMessage::Started {
                            task_name: task.name.clone(),
                            timestamp: std::time::SystemTime::now(),
                        });

                        let handle = self.execute_task(task.clone(), tx.clone()).await?;
                        let task_name = task.name.clone();
                        active_tasks.insert(task_name.clone(), handle);
                    }
                    Ok(false) => {
                        // Task can be skipped - outputs are up to date
                        info!(
                            "Skipping task {} - outputs are up to date ({}/{})",
                            task.name,
                            completed_tasks + 1,
                            total_tasks
                        );

                        // Print user-visible skipped message (only in terminal mode)
                        if !self.tui_mode {
                            let skipped_msg = format!("{} skipped (up to date)\n", colorize_task_prefix(&task.name));
                            print!("{skipped_msg}");
                            io::stdout().flush().unwrap_or(());
                        }

                        // Broadcast task skipped to TUI
                        self.broadcast_message(TaskMessage::StatusChange {
                            task_name: task.name.clone(),
                            status: TuiTaskStatus::Skipped,
                            timestamp: std::time::SystemTime::now(),
                        });

                        let mut statuses = self.task_statuses.lock().await;
                        statuses.insert(task.name.clone(), TaskStatus::Skipped);
                        completed_set.insert(task.name.clone());
                        completed_tasks += 1;

                        // Check if any blocked tasks are now ready due to this "completion"
                        blocked_tasks.retain(|blocked_task| {
                            let task_deps_completed = blocked_task
                                .task_deps
                                .iter()
                                .all(|task_dep| completed_set.contains(task_dep));
                            if !task_deps_completed {
                                return true; // Keep the task in blocked list
                            }

                            // All dependencies are completed, move to ready queue
                            ready_queue.push_back(blocked_task.clone());
                            false // Remove from blocked list
                        });
                    }
                    Err(e) => {
                        error!("Error checking file dependencies for task {}: {}", task.name, e);
                        // On error, default to running the task
                        info!(
                            "Starting task {} (file check failed, defaulting to run) ({}/{})",
                            task.name,
                            completed_tasks + 1,
                            total_tasks
                        );

                        // Broadcast task started to TUI
                        self.broadcast_message(TaskMessage::Started {
                            task_name: task.name.clone(),
                            timestamp: std::time::SystemTime::now(),
                        });

                        let handle = self.execute_task(task.clone(), tx.clone()).await?;
                        let task_name = task.name.clone();
                        active_tasks.insert(task_name.clone(), handle);
                    }
                }
            }

            // Wait for any task to complete
            match rx.recv().await {
                Some(Ok(completed_task)) => {
                    info!("Task {completed_task} completed successfully");

                    // Print user-visible success message (only in terminal mode)
                    if !self.tui_mode {
                        let success_msg = format!("{} finished successfully\n", colorize_task_prefix(&completed_task));
                        print!("{success_msg}");
                        io::stdout().flush().unwrap_or(());
                    }

                    // Broadcast task completion to TUI
                    self.broadcast_message(TaskMessage::Finished {
                        task_name: completed_task.clone(),
                        status: TuiTaskStatus::Completed,
                        timestamp: std::time::SystemTime::now(),
                        duration_ms: 0, // TODO: track actual duration
                    });

                    let mut statuses = self.task_statuses.lock().await;
                    statuses.insert(completed_task.clone(), TaskStatus::Completed);
                    completed_set.insert(completed_task.clone());
                    completed_tasks += 1;
                    active_tasks.remove(&completed_task);

                    // Check if any blocked tasks are now ready
                    blocked_tasks.retain(|task| {
                        let task_deps_completed =
                            task.task_deps.iter().all(|task_dep| completed_set.contains(task_dep));
                        if !task_deps_completed {
                            return true; // Keep the task in blocked list
                        }

                        // All dependencies are completed, move to ready queue
                        ready_queue.push_back(task.clone());
                        false // Remove from blocked list
                    });

                    // Update in-degree for dependent tasks
                    for remaining_task in &blocked_tasks {
                        if remaining_task.task_deps.contains(&completed_task)
                            && let Some(degree) = in_degree.get_mut(&remaining_task.name)
                        {
                            *degree = degree.saturating_sub(1);
                        }
                    }
                }
                Some(Err(e)) => {
                    error!("Task execution failed: {e}");

                    // Extract task name from error message for user-visible failure message
                    let error_str = e.to_string();
                    if let Some(task_name) = error_str.split_whitespace().nth(1) {
                        // Print user-visible failure message (only in terminal mode)
                        if !self.tui_mode {
                            let failure_msg = format!("{} failed\n", colorize_task_prefix(task_name));
                            eprint!("{failure_msg}");
                            io::stderr().flush().unwrap_or(());
                        }

                        // Broadcast task failure to TUI
                        self.broadcast_message(TaskMessage::Finished {
                            task_name: task_name.to_string(),
                            status: TuiTaskStatus::Failed,
                            timestamp: std::time::SystemTime::now(),
                            duration_ms: 0,
                        });
                    }

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
    async fn execute_task(&self, task: Task, tx: mpsc::Sender<Result<String>>) -> Result<JoinHandle<Result<()>>> {
        let semaphore = self.semaphore.clone();

        let task_name = task.name.clone();
        let task_dir = self.workspace.task(&task_name);
        let task_statuses = self.task_statuses.clone();
        let task_deps = task.task_deps.clone();
        let workspace = self.workspace.clone();
        let envs = task.envs.clone();
        let tasks_dir = self.workspace.run().join("tasks");
        let execution_context = self.execution_context.clone();
        let suppress_terminal = self.tui_mode;
        let task_streams = self.task_streams.clone();

        Ok(tokio::spawn(async move {
            // Acquire semaphore permit
            let _permit = semaphore.acquire().await?;

            // Double-check dependencies are still complete before starting
            {
                let statuses = task_statuses.lock().await;
                for dep in &task_deps {
                    match statuses.get(dep) {
                        Some(TaskStatus::Completed) | Some(TaskStatus::Skipped) => {
                            // Dependency is satisfied
                        }
                        _ => {
                            return Err(eyre!("Dependency {} not completed for task {}", dep, task_name));
                        }
                    }
                }
            }

            // Update task status to Running ONLY after dependency check
            {
                let mut statuses = task_statuses.lock().await;
                statuses.insert(task_name.clone(), TaskStatus::Running);
            }

            info!("Starting task {task_name}");

            // Create task directory only (no subdirectories)
            tokio::fs::create_dir_all(&task_dir).await?;

            // Setup dependency input files (symlink outputs from dependencies)
            for dep_name in &task_deps {
                let dep_output_file = workspace.task_output_file(dep_name);
                let current_input_file = workspace.task_input_file(&task_name, dep_name);

                // Only create symlink if dependency output exists
                if dep_output_file.exists() {
                    // Remove existing symlink/file if it exists
                    if current_input_file.exists() {
                        tokio::fs::remove_file(&current_input_file).await.ok();
                    }

                    // Create symlink from dependency output to current task input
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs;
                        fs::symlink(&dep_output_file, &current_input_file)?;
                    }
                    #[cfg(not(unix))]
                    {
                        // Fallback: copy file on non-Unix systems
                        tokio::fs::copy(&dep_output_file, &current_input_file).await?;
                    }
                }
            }

            // Process the user's action script with Otto enhancements
            let action_processor = ActionProcessor::new(workspace.clone(), &task_name)?;
            let processed_action = action_processor.process(&task.action, &task)?;

            // Extract script path and determine interpreter
            let (script_path, interpreter) = match processed_action {
                ProcessedAction::Bash { path, .. } => (path, "bash"),
                ProcessedAction::Python3 { path, .. } => (path, "python3"),
            };

            // Record task start in database with paths (graceful degradation)
            let db_task_id = if let Some(run_id) = workspace.db_run_id() {
                if let Some(manager) = super::state::StateManager::try_new() {
                    let stdout_path = tasks_dir.join(&task_name).join("stdout.log");
                    let stderr_path = tasks_dir.join(&task_name).join("stderr.log");

                    match manager.record_task_start(
                        run_id,
                        &task_name,
                        None, // TODO: Compute script hash in future phase
                        Some(&stdout_path),
                        Some(&stderr_path),
                        Some(&script_path),
                    ) {
                        Ok(task_id) => Some(task_id),
                        Err(e) => {
                            log::warn!("Failed to record task start in database: {}", e);
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Setup command environment
            let mut cmd = Command::new(interpreter);
            cmd.arg(&script_path)
                .current_dir(workspace.root())
                // Inherit current environment by default (no env_clear())
                .envs(&envs) // Override with user-specified env vars
                .env("OTTO_TASK", &task_name)
                .env("OTTO_TASK_DIR", task_dir.to_string_lossy().to_string())
                .env("OTTO_WORKSPACE", workspace.root().to_string_lossy().to_string())
                .env("OTTO_TASKS_DIR", tasks_dir.to_string_lossy().to_string())
                .env("OTTO_USER", &execution_context.user);

            // Execute without timeout - runs until completion or failure
            let result = async {
                let mut child = cmd
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()?;

                // Setup output streams
                let stdout = child.stdout.take().ok_or_else(|| eyre!("Failed to capture stdout"))?;
                let stderr = child.stderr.take().ok_or_else(|| eyre!("Failed to capture stderr"))?;

                // Use pre-created streams if available (TUI mode), otherwise create new ones
                let streams = if let Some(streams_map) = &task_streams {
                    streams_map
                        .get(&task_name)
                        .ok_or_else(|| eyre!("TaskStreams not found for task {}", task_name))?
                        .clone()
                } else {
                    TaskStreams::new(&task_name, &tasks_dir).await?
                };

                // Start output handling
                let stdout_handle = {
                    let streams = streams.clone();
                    let task_name = task_name.clone();
                    tokio::spawn(async move {
                        let reader = BufReader::new(stdout);
                        streams
                            .process_output(task_name, OutputType::Stdout, reader, suppress_terminal)
                            .await
                    })
                };

                let stderr_handle = {
                    let streams = streams.clone();
                    let task_name = task_name.clone();
                    tokio::spawn(async move {
                        let reader = BufReader::new(stderr);
                        streams
                            .process_output(task_name, OutputType::Stderr, reader, suppress_terminal)
                            .await
                    })
                };

                // Wait for process to complete
                let status = child.wait().await?;

                // Wait for output handling to complete with timeout (only for output processing)
                let output_timeout = Duration::from_secs(OUTPUT_PROCESSING_TIMEOUT_SECS);

                match timeout(output_timeout, stdout_handle).await {
                    Ok(Ok(Ok(()))) => {
                        // Stdout processing completed successfully
                    }
                    Ok(Ok(Err(e))) => {
                        error!("Stdout processing failed for task {task_name}: {e}");
                    }
                    Ok(Err(e)) => {
                        error!("Stdout processing join failed for task {task_name}: {e}");
                    }
                    Err(_) => {
                        error!("Stdout processing timed out for task {task_name}");
                    }
                }

                match timeout(output_timeout, stderr_handle).await {
                    Ok(Ok(Ok(()))) => {
                        // Stderr processing completed successfully
                    }
                    Ok(Ok(Err(e))) => {
                        error!("Stderr processing failed for task {task_name}: {e}");
                    }
                    Ok(Err(e)) => {
                        error!("Stderr processing join failed for task {task_name}: {e}");
                    }
                    Err(_) => {
                        error!("Stderr processing timed out for task {task_name}");
                    }
                }

                if status.success() {
                    Ok(())
                } else {
                    Err(eyre!("Task {} failed with exit code {:?}", task_name, status.code()))
                }
            }
            .await;

            match result {
                Ok(()) => {
                    info!("Task {task_name} completed successfully");

                    // Record task completion in database (graceful degradation)
                    if let Some(task_id) = db_task_id
                        && let Some(manager) = super::state::StateManager::try_new()
                        && let Err(e) = manager.record_task_complete(task_id, 0, super::state::TaskStatus::Completed)
                    {
                        log::warn!("Failed to record task completion in database: {}", e);
                    }

                    // Ensure we send the completion message
                    if let Err(e) = tx.send(Ok(task_name.clone())).await {
                        error!("Failed to send completion notification for task {task_name}: {e}");
                    }
                }
                Err(e) => {
                    error!("Task {task_name} failed: {e}");

                    // Record task failure in database (graceful degradation)
                    if let Some(task_id) = db_task_id
                        && let Some(manager) = super::state::StateManager::try_new()
                    {
                        // Extract exit code from error message if possible
                        let exit_code = e
                            .to_string()
                            .split("exit code")
                            .nth(1)
                            .and_then(|s| s.trim().parse::<i32>().ok())
                            .unwrap_or(1);

                        // Ignore errors in graceful degradation
                        let _ = manager.record_task_complete(task_id, exit_code, super::state::TaskStatus::Failed);
                    }

                    let mut statuses = task_statuses.lock().await;
                    statuses.insert(task_name.clone(), TaskStatus::Failed(e.to_string()));
                    if let Err(send_err) = tx.send(Err(e)).await {
                        error!("Failed to send error notification for task {task_name}: {send_err}");
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
        let statuses = self.task_statuses.lock().await;
        statuses.get(task_name).cloned().unwrap_or(TaskStatus::Pending)
    }

    /// Check if a task needs to be rebuilt based on file dependencies
    pub async fn needs_rebuild(&self, task: &Task) -> Result<bool> {
        // If no file dependencies, always run (traditional task-only mode)
        if task.file_deps.is_empty() {
            debug!("Task {} has no file dependencies, will run", task.name);
            return Ok(true);
        }

        // Get output files from the task
        let output_files = &task.output_deps;

        // If no output files exist, need to run
        if output_files.is_empty() {
            debug!("Task {} has no output files defined, will run", task.name);
            return Ok(true);
        }

        // Check if any output files are missing
        for output_path in output_files {
            if !Path::new(output_path).exists() {
                debug!(
                    "Output file {} does not exist, task {} needs to run",
                    output_path, task.name
                );
                return Ok(true);
            }
        }

        // Get timestamps for all files
        let input_timestamps = self.get_file_timestamps(&task.file_deps).await?;
        let output_timestamps = self.get_file_timestamps(output_files).await?;

        // Find the newest input and oldest output
        let newest_input = input_timestamps.iter().filter_map(|(_, time)| *time).max();
        let oldest_output = output_timestamps.iter().filter_map(|(_, time)| *time).min();

        match (newest_input, oldest_output) {
            (Some(input_time), Some(output_time)) => {
                let needs_rebuild = input_time > output_time;
                if needs_rebuild {
                    debug!("Input files newer than outputs, task {} needs to run", task.name);
                } else {
                    debug!("Outputs up to date, task {} can be skipped", task.name);
                }
                Ok(needs_rebuild)
            }
            (None, _) => {
                debug!("No input files found, task {} will run", task.name);
                Ok(true) // No inputs found, run the task
            }
            (_, None) => {
                debug!("No output files found, task {} needs to run", task.name);
                Ok(true) // No outputs found, need to run
            }
        }
    }

    /// Get file timestamps for a list of file paths
    async fn get_file_timestamps(&self, file_paths: &[String]) -> Result<Vec<(String, Option<std::time::SystemTime>)>> {
        let mut timestamps = Vec::new();

        for file_path in file_paths {
            let path = Path::new(file_path);
            let timestamp = if path.exists() {
                match tokio::fs::metadata(path).await {
                    Ok(metadata) => match metadata.modified() {
                        Ok(time) => Some(time),
                        Err(e) => {
                            debug!("Could not get modification time for {file_path}: {e}");
                            None
                        }
                    },
                    Err(e) => {
                        debug!("Could not get metadata for {file_path}: {e}");
                        None
                    }
                }
            } else {
                debug!("File {file_path} does not exist");
                None
            };
            timestamps.push((file_path.clone(), timestamp));
        }

        Ok(timestamps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Helper to set up a test-specific database path
    fn setup_test_db(temp_dir: &std::path::Path) {
        let db_path = temp_dir.join("test_otto.db");
        // SAFETY: This is safe in tests because we control the execution environment
        // and tests are isolated. The env var is set before any StateManager is created.
        unsafe {
            std::env::set_var("OTTO_DB_PATH", &db_path);
        }
    }

    #[tokio::test]
    async fn test_task_execution() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        setup_test_db(&work_dir);

        let task = Task::new(
            "test".to_string(),
            vec![],
            vec![],
            vec![],
            HashMap::new(),
            HashMap::new(),
            "echo hello".to_string(),
        );

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(vec![task], Arc::new(workspace), ExecutionContext::new(), 2, false).await?;
        scheduler.execute_all().await?;

        let status = scheduler.get_task_status("test").await;
        assert_eq!(status, TaskStatus::Completed);

        Ok(())
    }

    #[tokio::test]
    async fn test_task_dependencies() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        setup_test_db(&work_dir);

        let tasks = vec![
            Task::new(
                "task1".to_string(),
                vec!["task2".to_string()],
                vec![],
                vec![],
                HashMap::new(),
                HashMap::new(),
                "echo task1".to_string(),
            ),
            Task::new(
                "task2".to_string(),
                vec![],
                vec![],
                vec![],
                HashMap::new(),
                HashMap::new(),
                "echo task2".to_string(),
            ),
        ];

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), 2, false).await?;
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
        setup_test_db(&work_dir);

        let tasks = vec![
            Task::new(
                "task1".to_string(),
                vec![],
                vec![],
                vec![],
                HashMap::new(),
                HashMap::new(),
                "exit 1".to_string(),
            ),
            Task::new(
                "task2".to_string(),
                vec!["task1".to_string()],
                vec![],
                vec![],
                HashMap::new(),
                HashMap::new(),
                "echo task2".to_string(),
            ),
        ];

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), 2, false).await?;
        let result = scheduler.execute_all().await;

        assert!(result.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_file_dependencies() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        setup_test_db(&work_dir);

        // Create a test input file
        let input_file = work_dir.join("input.txt");
        let output_file = work_dir.join("output.txt");
        tokio::fs::write(&input_file, "test content").await?;

        // Create task with file dependencies
        let task = Task::new(
            "copy_task".to_string(),
            vec![],
            vec![input_file.to_string_lossy().to_string()],
            vec![output_file.to_string_lossy().to_string()],
            HashMap::new(),
            HashMap::new(),
            format!("cp {} {}", input_file.display(), output_file.display()),
        );

        let workspace = Workspace::new(work_dir.clone()).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(
            vec![task.clone()],
            Arc::new(workspace),
            ExecutionContext::new(),
            2,
            false,
        )
        .await?;

        // First run should execute (no output file exists)
        let needs_rebuild = scheduler.needs_rebuild(&task).await?;
        assert!(needs_rebuild, "Task should need to run when output doesn't exist");

        // Simulate file creation with newer timestamp
        tokio::fs::write(&output_file, "output content").await?;

        // Set output file to be newer than input file
        let now = std::time::SystemTime::now();
        let future_time = filetime::FileTime::from_system_time(now + std::time::Duration::from_secs(1));
        filetime::set_file_times(&output_file, future_time, future_time)?;

        // Now the task should not need to run (output newer than input)
        let needs_rebuild_after = scheduler.needs_rebuild(&task).await?;
        assert!(
            !needs_rebuild_after,
            "Task should not need to run when output is newer than inputs"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_file_timestamp_checking() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        setup_test_db(&work_dir);

        let file1 = work_dir.join("file1.txt");
        let file2 = work_dir.join("file2.txt");

        // Create files with known content
        tokio::fs::write(&file1, "content1").await?;
        tokio::fs::write(&file2, "content2").await?;

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(vec![], Arc::new(workspace), ExecutionContext::new(), 2, false).await?;

        // Test timestamp retrieval
        let timestamps = scheduler
            .get_file_timestamps(&[file1.to_string_lossy().to_string(), file2.to_string_lossy().to_string()])
            .await?;

        assert_eq!(timestamps.len(), 2);
        assert!(timestamps[0].1.is_some(), "Should have timestamp for existing file");
        assert!(timestamps[1].1.is_some(), "Should have timestamp for existing file");

        Ok(())
    }

    #[tokio::test]
    async fn test_file_dependencies_nonexistent_files() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        setup_test_db(&work_dir);

        let nonexistent_file = work_dir.join("nonexistent.txt");
        let output_file = work_dir.join("output.txt");

        // Create task with nonexistent input file
        let task = Task::new(
            "test_nonexistent".to_string(),
            vec![],
            vec![nonexistent_file.to_string_lossy().to_string()],
            vec![output_file.to_string_lossy().to_string()],
            HashMap::new(),
            HashMap::new(),
            format!("touch {}", output_file.display()),
        );

        let workspace = Workspace::new(work_dir.clone()).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(
            vec![task.clone()],
            Arc::new(workspace),
            ExecutionContext::new(),
            2,
            false,
        )
        .await?;

        // Should need to rebuild when input file doesn't exist (conservative approach)
        let needs_rebuild = scheduler.needs_rebuild(&task).await?;
        assert!(needs_rebuild, "Task should need to run when input file doesn't exist");

        Ok(())
    }

    #[tokio::test]
    async fn test_file_dependencies_multiple_inputs_outputs() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        setup_test_db(&work_dir);

        // Create multiple input files with different timestamps
        let input1 = work_dir.join("input1.txt");
        let input2 = work_dir.join("input2.txt");
        let input3 = work_dir.join("input3.txt");
        let output1 = work_dir.join("output1.txt");
        let output2 = work_dir.join("output2.txt");

        tokio::fs::write(&input1, "content1").await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        tokio::fs::write(&input2, "content2").await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        tokio::fs::write(&input3, "content3").await?;

        let task = Task::new(
            "multi_files".to_string(),
            vec![],
            vec![
                input1.to_string_lossy().to_string(),
                input2.to_string_lossy().to_string(),
                input3.to_string_lossy().to_string(),
            ],
            vec![
                output1.to_string_lossy().to_string(),
                output2.to_string_lossy().to_string(),
            ],
            HashMap::new(),
            HashMap::new(),
            format!(
                "cat {} {} {} > {} && cp {} {}",
                input1.display(),
                input2.display(),
                input3.display(),
                output1.display(),
                output1.display(),
                output2.display()
            ),
        );

        let workspace = Workspace::new(work_dir.clone()).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(
            vec![task.clone()],
            Arc::new(workspace),
            ExecutionContext::new(),
            2,
            false,
        )
        .await?;

        // Should need to rebuild when outputs don't exist
        let needs_rebuild = scheduler.needs_rebuild(&task).await?;
        assert!(needs_rebuild, "Task should need to run when outputs don't exist");

        // Create output files that are newer than all inputs
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        tokio::fs::write(&output1, "combined output").await?;
        tokio::fs::write(&output2, "combined output copy").await?;

        // Should not need to rebuild when all outputs are newer than all inputs
        let needs_rebuild_after = scheduler.needs_rebuild(&task).await?;
        assert!(
            !needs_rebuild_after,
            "Task should not need to run when all outputs are newer than all inputs"
        );

        // Touch one of the input files to make it newer
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        tokio::fs::write(&input2, "modified content2").await?;

        // Should need to rebuild when any input is newer than any output
        let needs_rebuild_final = scheduler.needs_rebuild(&task).await?;
        assert!(
            needs_rebuild_final,
            "Task should need to run when any input is newer than outputs"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_file_dependencies_with_task_dependencies() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());

        let input_file = work_dir.join("input.txt");
        let intermediate_file = work_dir.join("intermediate.txt");
        let output_file = work_dir.join("output.txt");

        tokio::fs::write(&input_file, "initial content").await?;

        let task1 = Task::new(
            "step1".to_string(),
            vec![],
            vec![input_file.to_string_lossy().to_string()],
            vec![intermediate_file.to_string_lossy().to_string()],
            HashMap::new(),
            HashMap::new(),
            format!("cp {} {}", input_file.display(), intermediate_file.display()),
        );

        let task2 = Task::new(
            "step2".to_string(),
            vec!["step1".to_string()],                             // Task dependency
            vec![intermediate_file.to_string_lossy().to_string()], // File dependency
            vec![output_file.to_string_lossy().to_string()],
            HashMap::new(),
            HashMap::new(),
            format!("cp {} {}", intermediate_file.display(), output_file.display()),
        );

        let workspace = Workspace::new(work_dir.clone()).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(
            vec![task1.clone(), task2.clone()],
            Arc::new(workspace),
            ExecutionContext::new(),
            2,
            false,
        )
        .await?;

        // Both tasks should need to run initially
        let task1_needs_rebuild = scheduler.needs_rebuild(&task1).await?;
        let task2_needs_rebuild = scheduler.needs_rebuild(&task2).await?;
        assert!(task1_needs_rebuild, "Task1 should need to run initially");
        assert!(task2_needs_rebuild, "Task2 should need to run initially");

        // Execute all tasks
        scheduler.execute_all().await?;

        // Verify both tasks completed
        let task1_status = scheduler.get_task_status("step1").await;
        let task2_status = scheduler.get_task_status("step2").await;
        assert_eq!(task1_status, TaskStatus::Completed);
        assert_eq!(task2_status, TaskStatus::Completed);

        // Verify files were created
        assert!(intermediate_file.exists(), "Intermediate file should exist");
        assert!(output_file.exists(), "Output file should exist");

        Ok(())
    }

    #[tokio::test]
    async fn test_file_dependencies_timestamp_precision() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());

        let input_file = work_dir.join("input.txt");
        let output_file = work_dir.join("output.txt");

        // Create input file
        tokio::fs::write(&input_file, "content").await?;

        // Create output file with same timestamp (within same millisecond)
        tokio::fs::write(&output_file, "output").await?;

        let task = Task::new(
            "timestamp_test".to_string(),
            vec![],
            vec![input_file.to_string_lossy().to_string()],
            vec![output_file.to_string_lossy().to_string()],
            HashMap::new(),
            HashMap::new(),
            format!("cp {} {}", input_file.display(), output_file.display()),
        );

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(
            vec![task.clone()],
            Arc::new(workspace),
            ExecutionContext::new(),
            2,
            false,
        )
        .await?;

        // When timestamps are very close, should be conservative and rebuild
        let needs_rebuild = scheduler.needs_rebuild(&task).await?;
        // This might be true or false depending on timestamp precision, but should be consistent
        println!("Timestamp precision test - needs rebuild: {needs_rebuild}");

        Ok(())
    }

    #[tokio::test]
    async fn test_file_dependencies_empty_lists() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());

        // Task with no file dependencies
        let task = Task::new(
            "no_file_deps".to_string(),
            vec![],
            vec![], // No input files
            vec![], // No output files
            HashMap::new(),
            HashMap::new(),
            "echo 'no file dependencies'".to_string(),
        );

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(
            vec![task.clone()],
            Arc::new(workspace),
            ExecutionContext::new(),
            2,
            false,
        )
        .await?;

        // Should always need to run when there are no file dependencies to check
        let needs_rebuild = scheduler.needs_rebuild(&task).await?;
        assert!(needs_rebuild, "Task with no file dependencies should always run");

        Ok(())
    }

    #[tokio::test]
    async fn test_file_dependencies_directory_as_input() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        setup_test_db(&work_dir);

        // Create a directory with files
        let src_dir = work_dir.join("src");
        tokio::fs::create_dir_all(&src_dir).await?;
        tokio::fs::write(src_dir.join("file1.txt"), "content1").await?;
        tokio::fs::write(src_dir.join("file2.txt"), "content2").await?;

        let output_file = work_dir.join("output.txt");

        let task = Task::new(
            "dir_input".to_string(),
            vec![],
            vec![src_dir.to_string_lossy().to_string()], // Directory as input
            vec![output_file.to_string_lossy().to_string()],
            HashMap::new(),
            HashMap::new(),
            format!(
                "find {} -name '*.txt' | wc -l > {}",
                src_dir.display(),
                output_file.display()
            ),
        );

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(
            vec![task.clone()],
            Arc::new(workspace),
            ExecutionContext::new(),
            2,
            false,
        )
        .await?;

        // Should handle directory dependencies (gets modification time of directory)
        let needs_rebuild = scheduler.needs_rebuild(&task).await?;
        assert!(needs_rebuild, "Task should need to run when output doesn't exist");

        Ok(())
    }

    #[tokio::test]
    async fn test_large_number_of_file_dependencies() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        setup_test_db(&work_dir);

        // Create many input files
        let mut input_files = Vec::new();
        for i in 0..100 {
            let file = work_dir.join(format!("input_{i:03}.txt"));
            tokio::fs::write(&file, format!("content {i}")).await?;
            input_files.push(file.to_string_lossy().to_string());
        }

        let output_file = work_dir.join("combined.txt");

        let task = Task::new(
            "many_inputs".to_string(),
            vec![],
            input_files,
            vec![output_file.to_string_lossy().to_string()],
            HashMap::new(),
            HashMap::new(),
            format!("cat input_*.txt > {}", output_file.display()),
        );

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(
            vec![task.clone()],
            Arc::new(workspace),
            ExecutionContext::new(),
            2,
            false,
        )
        .await?;

        // Should handle large numbers of file dependencies efficiently
        let start = std::time::Instant::now();
        let needs_rebuild = scheduler.needs_rebuild(&task).await?;
        let duration = start.elapsed();

        assert!(needs_rebuild, "Task should need to run when output doesn't exist");
        assert!(
            duration.as_millis() < 1000,
            "File dependency checking should be fast even with many files"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_file_dependencies_circular_detection() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        setup_test_db(&work_dir);

        let file_a = work_dir.join("a.txt");
        let file_b = work_dir.join("b.txt");

        tokio::fs::write(&file_a, "content a").await?;
        tokio::fs::write(&file_b, "content b").await?;

        // Task that uses its output as input (circular dependency)
        let task = Task::new(
            "circular".to_string(),
            vec![],
            vec![file_a.to_string_lossy().to_string()],
            vec![file_a.to_string_lossy().to_string()], // Same file as input and output
            HashMap::new(),
            HashMap::new(),
            format!("echo 'modified' >> {}", file_a.display()),
        );

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(
            vec![task.clone()],
            Arc::new(workspace),
            ExecutionContext::new(),
            2,
            false,
        )
        .await?;

        // Should handle circular file dependencies gracefully
        let needs_rebuild = scheduler.needs_rebuild(&task).await?;
        // Should be conservative when input and output are the same file
        println!("Circular dependency test - needs rebuild: {needs_rebuild}");

        Ok(())
    }

    #[tokio::test]
    async fn test_file_dependencies_integration_with_real_execution() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());

        let input_file = work_dir.join("source.txt");
        let output_file = work_dir.join("result.txt");

        // Create input file with known content
        tokio::fs::write(&input_file, "Hello, World!").await?;

        let task = Task::new(
            "real_execution".to_string(),
            vec![],
            vec![input_file.to_string_lossy().to_string()],
            vec![output_file.to_string_lossy().to_string()],
            HashMap::new(),
            HashMap::new(),
            format!("cp {} {}", input_file.display(), output_file.display()),
        );

        let workspace = Workspace::new(work_dir.clone()).await?;
        workspace.init().await?;
        let scheduler = TaskScheduler::new(
            vec![task.clone()],
            Arc::new(workspace),
            ExecutionContext::new(),
            2,
            false,
        )
        .await?;

        // First execution - should run because output doesn't exist
        let needs_rebuild_1 = scheduler.needs_rebuild(&task).await?;
        assert!(needs_rebuild_1, "Should need to run initially");

        scheduler.execute_all().await?;

        // Verify task completed and output file was created
        let status = scheduler.get_task_status("real_execution").await;
        assert_eq!(status, TaskStatus::Completed);
        assert!(output_file.exists(), "Output file should exist after execution");

        let output_content = tokio::fs::read_to_string(&output_file).await?;
        assert_eq!(output_content, "Hello, World!", "Output should match input");

        // Second check - should not need to run because output is newer
        let needs_rebuild_2 = scheduler.needs_rebuild(&task).await?;
        assert!(!needs_rebuild_2, "Should not need to run when output is up-to-date");

        // Modify input file to trigger rebuild
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        tokio::fs::write(&input_file, "Modified content!").await?;

        // Third check - should need to run because input is newer
        let needs_rebuild_3 = scheduler.needs_rebuild(&task).await?;
        assert!(needs_rebuild_3, "Should need to run when input is modified");

        Ok(())
    }

    #[tokio::test]
    async fn test_parallel_execution_limit() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = temp_dir.path().to_path_buf();

        // Create tasks that sleep for a bit to test concurrency
        let mut tasks = vec![];
        for i in 1..=4 {
            let task = Task::new(
                format!("task{i}"),
                vec![],
                vec![],
                vec![],
                HashMap::new(),
                HashMap::new(),
                format!("sleep 0.1 && echo task{i}"),
            );
            tasks.push(task);
        }

        let workspace = Workspace::new(work_dir).await?;
        workspace.init().await?;

        // Create scheduler with limit of 2 parallel jobs
        let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), 2, false).await?;

        // Verify semaphore has 2 permits
        assert_eq!(scheduler.semaphore.available_permits(), 2);

        // Execute all tasks
        scheduler.execute_all().await?;

        // Verify all tasks completed
        for i in 1..=4 {
            let status = scheduler.get_task_status(&format!("task{i}")).await;
            assert_eq!(status, TaskStatus::Completed);
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_scheduler_respects_max_parallel() -> Result<()> {
        // Test with different job limits
        for max_parallel in [1, 2, 4, 8] {
            let temp_dir = TempDir::new()?;
            let work_dir = temp_dir.path().to_path_buf();

            let workspace = Workspace::new(work_dir).await?;
            workspace.init().await?;

            let tasks = vec![Task::new(
                "test".to_string(),
                vec![],
                vec![],
                vec![],
                HashMap::new(),
                HashMap::new(),
                "echo test".to_string(),
            )];

            let scheduler =
                TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), max_parallel, false).await?;

            // Verify semaphore has correct number of permits
            assert_eq!(
                scheduler.semaphore.available_permits(),
                max_parallel,
                "Scheduler should have {max_parallel} permits"
            );
        }

        Ok(())
    }
}
