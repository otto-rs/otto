use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use eyre::{eyre, Result};
use tokio::{
    process::Command,
    sync::{mpsc, Mutex, Semaphore},
    task::JoinHandle,
    time::timeout,
};
use tracing::{error, info};

use super::{
    output::TaskStreams,
    task::{Task, TaskStatus, TaskType},
};

/// Task scheduler that manages concurrent execution
pub struct TaskScheduler {
    /// Task status tracking
    task_statuses: Arc<Mutex<HashMap<String, TaskStatus>>>,
    /// Semaphore for I/O task limiting
    io_semaphore: Arc<Semaphore>,
    /// Semaphore for CPU task limiting
    cpu_semaphore: Arc<Semaphore>,
    /// Working directory for tasks
    work_dir: PathBuf,
    /// Task output streams
    pub streams: Arc<TaskStreams>,
    /// Tasks to execute
    tasks: Vec<Task>,
}

impl TaskScheduler {
    pub async fn new(
        tasks: Vec<Task>,
        work_dir: PathBuf,
        io_limit: usize,
        cpu_limit: usize,
    ) -> Result<Self> {
        let streams = TaskStreams::new("default", &work_dir).await?;

        Ok(Self {
            task_statuses: Arc::new(Mutex::new(HashMap::new())),
            io_semaphore: Arc::new(Semaphore::new(io_limit)),
            cpu_semaphore: Arc::new(Semaphore::new(cpu_limit)),
            work_dir,
            streams: Arc::new(streams),
            tasks,
        })
    }

    /// Execute all tasks in the graph
    pub async fn execute_all(&self) -> Result<()> {
        let (tx, mut rx) = mpsc::channel(32);

        // Initialize task statuses
        {
            let mut statuses = self.task_statuses.lock().await;
            for task in &self.tasks {
                statuses.insert(task.spec.name.clone(), TaskStatus::Pending);
            }
        }

        // Initialize in-degree counts for each task
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for task in &self.tasks {
            in_degree.insert(task.spec.name.clone(), task.spec.deps.len());
        }

        // Initialize ready queue with tasks that have no dependencies
        let mut ready_queue = std::collections::VecDeque::new();
        for task in &self.tasks {
            if task.spec.deps.is_empty() {
                ready_queue.push_back(task);
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
                let deps_completed = task.spec.deps.iter().all(|dep| completed_set.contains(dep));
                if !deps_completed {
                    // Put it back at the end of the queue
                    ready_queue.push_back(task);

                    // If we can't start any tasks in the queue, wait for completions
                    if ready_queue.len() == 1 {
                        break;
                    }
                    continue;
                }

                info!("Starting task {} ({}/{})", task.spec.name, completed_tasks + 1, total_tasks);

                let handle = self.execute_task(task.clone(), tx.clone()).await?;
                let task_name = task.spec.name.clone();
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

                    // Find tasks that are now ready
                    for task in &self.tasks {
                        if task.spec.deps.contains(&completed_task) {
                            let degree = in_degree.get_mut(&task.spec.name).unwrap();
                            *degree -= 1;
                            if *degree == 0 && !completed_set.contains(&task.spec.name) {
                                ready_queue.push_back(task);
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
        task: Task,
        tx: mpsc::Sender<Result<String>>,
    ) -> Result<JoinHandle<Result<()>>> {
        let semaphore = match task.task_type {
            TaskType::IOBound | TaskType::NetworkBound => self.io_semaphore.clone(),
            TaskType::CPUBound => self.cpu_semaphore.clone(),
        };

        let task_name = task.spec.name.clone();
        let output_dir = task.output_dir.clone();
        let timeout_secs = task.spec.timeout;
        let task_statuses = self.task_statuses.clone();
        let deps = task.spec.deps.clone();

        Ok(tokio::spawn(async move {
            // Acquire semaphore permit
            let _permit = semaphore.acquire().await?;

            // Double-check dependencies are still complete before starting
            {
                let statuses = task_statuses.lock().await;
                for dep in &deps {
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

            // Execute the task with timeout
            let result = timeout(
                Duration::from_secs(timeout_secs),
                async {
                    let mut cmd = Command::new("sh")
                        .current_dir(&output_dir)
                        .arg("-c")
                        .arg(&task.spec.action)
                        .envs(&task.spec.envs)
                        .stdout(std::process::Stdio::inherit())
                        .stderr(std::process::Stdio::inherit())
                        .kill_on_drop(true)
                        .spawn()?;

                    // Wait for command completion
                    let status = cmd.wait().await?;

                    if !status.success() {
                        return Err(eyre!(
                            "Task {} failed with exit code {:?}",
                            task_name,
                            status.code()
                        ));
                    }
                    Ok(())
                }
            ).await;

            match result {
                Ok(Ok(_)) => {
                    // Send completion notification
                    tx.send(Ok(task_name.clone())).await?;
                    Ok(())
                }
                Ok(Err(e)) => {
                    let err_msg = format!("Task {} failed: {}", task_name, e);
                    tx.send(Err(eyre!(err_msg.clone()))).await?;
                    Err(eyre!(err_msg))
                }
                Err(_) => {
                    let err_msg = format!("Task {} timed out", task_name);
                    tx.send(Err(eyre!(err_msg.clone()))).await?;
                    Err(eyre!(err_msg))
                }
            }
        }))
    }

    /// Get the directory for a task
    pub fn get_task_dir(&self, task_name: &str) -> Result<PathBuf> {
        let task_dir = self.work_dir.join(".otto").join(task_name);
        std::fs::create_dir_all(&task_dir)?;
        Ok(task_dir)
    }

    /// Get the current status of a task
    pub async fn get_task_status(&self, task_name: &str) -> TaskStatus {
        let statuses = self.task_statuses.lock().await;
        statuses.get(task_name)
            .cloned()
            .unwrap_or(TaskStatus::Pending)
    }

    /// Get all task statuses
    pub async fn get_task_statuses(&self) -> HashMap<String, TaskStatus> {
        self.task_statuses.lock().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::task::TaskSpec;
    use std::time::Instant;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_single_task_execution() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());

        let task = TaskSpec {
            name: "test".to_string(),
            action: "echo hello".to_string(),
            deps: vec![],
            envs: HashMap::new(),
            working_dir: None,
            timeout: 10,
        };

        let task = Task::new(task, work_dir.clone());
        let scheduler = TaskScheduler::new(vec![task], work_dir, 2, 2).await?;
        scheduler.execute_all().await?;

        let status = scheduler.get_task_status("test").await;
        assert_eq!(status, TaskStatus::Completed);

        Ok(())
    }

    #[tokio::test]
    async fn test_dependency_ordering() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        let output_file = work_dir.join("output.txt");

        let tasks = vec![
            TaskSpec {
                name: "dep".to_string(),
                action: format!("echo 'dep' > {}", output_file.display()),
                deps: vec![],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
            TaskSpec {
                name: "main".to_string(),
                action: format!("echo 'main' >> {}", output_file.display()),
                deps: vec!["dep".to_string()],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
        ];

        let tasks = tasks.into_iter()
            .map(|spec| Task::new(spec, work_dir.clone()))
            .collect::<Vec<_>>();

        let scheduler = TaskScheduler::new(tasks, work_dir, 2, 2).await?;
        scheduler.execute_all().await?;

        // Verify execution order through file contents
        let output = std::fs::read_to_string(output_file)?;
        assert_eq!(output.lines().collect::<Vec<_>>(), vec!["dep", "main"]);

        Ok(())
    }

    #[tokio::test]
    async fn test_parallel_execution() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        let output_file = work_dir.join("parallel.txt");

        let tasks = vec![
            TaskSpec {
                name: "parallel1".to_string(),
                action: format!("sleep 0.5 && echo 'p1' >> {}", output_file.display()),
                deps: vec![],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
            TaskSpec {
                name: "parallel2".to_string(),
                action: format!("sleep 0.5 && echo 'p2' >> {}", output_file.display()),
                deps: vec![],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
            TaskSpec {
                name: "after".to_string(),
                action: format!("echo 'after' >> {}", output_file.display()),
                deps: vec!["parallel1".to_string(), "parallel2".to_string()],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
        ];

        let tasks = tasks.into_iter()
            .map(|spec| Task::new(spec, work_dir.clone()))
            .collect::<Vec<_>>();

        let start = Instant::now();
        let scheduler = TaskScheduler::new(tasks, work_dir, 2, 2).await?;
        scheduler.execute_all().await?;
        let duration = start.elapsed();

        // Both parallel tasks should complete in ~0.5 seconds, not 1 second
        assert!(duration.as_secs_f32() < 0.75);

        // Verify 'after' task ran after both parallel tasks
        let output = std::fs::read_to_string(output_file)?;
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("p"));
        assert!(lines[1].starts_with("p"));
        assert_eq!(lines[2], "after");

        Ok(())
    }

    #[tokio::test]
    async fn test_complex_dag() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let work_dir = PathBuf::from(temp_dir.path());
        let output_file = work_dir.join("dag.txt");

        let tasks = vec![
            TaskSpec {
                name: "start".to_string(),
                action: format!("echo 'start' > {}", output_file.display()),
                deps: vec![],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
            TaskSpec {
                name: "task1".to_string(),
                action: format!("sleep 0.1 && echo 'task1' >> {}", output_file.display()),
                deps: vec!["start".to_string()],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
            TaskSpec {
                name: "task2".to_string(),
                action: format!("sleep 0.1 && echo 'task2' >> {}", output_file.display()),
                deps: vec!["start".to_string()],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
            TaskSpec {
                name: "task3".to_string(),
                action: format!("sleep 0.2 && echo 'task3' >> {}", output_file.display()),
                deps: vec!["task1".to_string()],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
            TaskSpec {
                name: "task4".to_string(),
                action: format!("sleep 0.2 && echo 'task4' >> {}", output_file.display()),
                deps: vec!["task2".to_string()],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
            TaskSpec {
                name: "finish".to_string(),
                action: format!("echo 'finish' >> {}", output_file.display()),
                deps: vec!["task3".to_string(), "task4".to_string()],
                envs: HashMap::new(),
                working_dir: None,
                timeout: 10,
            },
        ];

        let tasks = tasks.into_iter()
            .map(|spec| Task::new(spec, work_dir.clone()))
            .collect::<Vec<_>>();

        let scheduler = TaskScheduler::new(tasks, work_dir, 2, 2).await?;
        scheduler.execute_all().await?;

        // Verify execution order
        let output = std::fs::read_to_string(output_file)?;
        let lines: Vec<_> = output.lines().collect();

        // First line must be start
        assert_eq!(lines[0], "start");

        // task1 and task2 can be in either order
        assert!(lines[1] == "task1" || lines[1] == "task2");
        assert!(lines[2] == "task1" || lines[2] == "task2");
        assert!(lines[1] != lines[2]);

        // task3 must come after task1
        let task1_pos = lines.iter().position(|&l| l == "task1").unwrap();
        let task3_pos = lines.iter().position(|&l| l == "task3").unwrap();
        assert!(task1_pos < task3_pos);

        // task4 must come after task2
        let task2_pos = lines.iter().position(|&l| l == "task2").unwrap();
        let task4_pos = lines.iter().position(|&l| l == "task4").unwrap();
        assert!(task2_pos < task4_pos);

        // finish must be last
        assert_eq!(lines.last().unwrap(), &"finish");

        Ok(())
    }
}
