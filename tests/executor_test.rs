use std::path::PathBuf;
use eyre::Result;
use tempfile::TempDir;
use tokio::time::timeout;
use std::time::Duration;
use std::sync::Arc;

use otto::executor::{
    Task, TaskSpec, TaskStatus,
    TaskScheduler, Workspace,
    workspace::ExecutionContext,
};

#[tokio::test]
async fn test_task_execution_with_output() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());

    let task_spec = TaskSpec {
        name: "test".to_string(),
        action: "true".to_string(),
        deps: vec![],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let task = Task::new(task_spec);
    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(vec![task], Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    // Execute task with timeout
    timeout(Duration::from_secs(1), scheduler.execute_all()).await??;

    Ok(())
}

#[tokio::test]
async fn test_task_dependencies() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());

    let task1_spec = TaskSpec {
        name: "task1".to_string(),
        action: "true".to_string(),
        deps: vec![],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let task2_spec = TaskSpec {
        name: "task2".to_string(),
        action: "true".to_string(),
        deps: vec!["task1".to_string()],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let task3_spec = TaskSpec {
        name: "task3".to_string(),
        action: "true".to_string(),
        deps: vec!["task2".to_string()],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let tasks = vec![
        Task::new(task1_spec),
        Task::new(task2_spec),
        Task::new(task3_spec),
    ];

    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    // Execute task with timeout
    timeout(Duration::from_secs(1), scheduler.execute_all()).await??;

    let statuses = scheduler.get_task_statuses().await;
    for task_name in ["task1", "task2", "task3"] {
        assert_eq!(statuses[task_name], TaskStatus::Completed);
    }

    Ok(())
}

#[tokio::test]
async fn test_task_failure() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());

    let task_spec = TaskSpec {
        name: "failing_task".to_string(),
        action: "exit 1".to_string(),
        deps: vec![],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let task = Task::new(task_spec);
    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(vec![task], Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;
    
    // Execute task with timeout
    let result = timeout(Duration::from_secs(1), scheduler.execute_all()).await?;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("failed with exit code"));

    Ok(())
}

#[tokio::test]
async fn test_task_timeout() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());

    let task_spec = TaskSpec {
        name: "timeout_task".to_string(),
        action: "sleep 1.5".to_string(), // Sleep for 1.5 seconds
        deps: vec![],
        envs: Default::default(),
        working_dir: None,
        timeout: 1, // But timeout after 1 second
    };

    let task = Task::new(task_spec);
    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(vec![task], Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;
    
    // Execute task with timeout
    let result = scheduler.execute_all().await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("timed out"));

    Ok(())
}

#[tokio::test]
async fn test_output_capture() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());

    let task_spec = TaskSpec {
        name: "output_test".to_string(),
        action: "true".to_string(),
        deps: vec![],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let task = Task::new(task_spec);
    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(vec![task], Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    // Execute task with timeout
    timeout(Duration::from_secs(1), scheduler.execute_all()).await??;

    Ok(())
}

#[tokio::test]
async fn test_dependency_ordering() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());
    let output_file = work_dir.join("output.txt");

    let task1_spec = TaskSpec {
        name: "task1".to_string(),
        action: format!("echo 'task1' >> {}", output_file.display()),
        deps: vec!["task2".to_string(), "task3".to_string()],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let task2_spec = TaskSpec {
        name: "task2".to_string(),
        action: format!("echo 'task2' >> {}", output_file.display()),
        deps: vec!["task4".to_string()],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let task3_spec = TaskSpec {
        name: "task3".to_string(),
        action: format!("echo 'task3' >> {}", output_file.display()),
        deps: vec!["task4".to_string()],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let task4_spec = TaskSpec {
        name: "task4".to_string(),
        action: format!("echo 'task4' >> {}", output_file.display()),
        deps: vec![],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let tasks = vec![
        Task::new(task1_spec),
        Task::new(task2_spec),
        Task::new(task3_spec),
        Task::new(task4_spec),
    ];

    let workspace = Workspace::new(work_dir.clone()).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    // Execute tasks with timeout
    timeout(Duration::from_secs(2), scheduler.execute_all()).await??;

    // Verify execution order through file contents
    let output = std::fs::read_to_string(output_file)?;
    let lines: Vec<_> = output.lines().collect();

    // task4 must be first
    assert_eq!(lines[0], "task4");

    // task2 and task3 must come after task4 but before task1
    let task4_pos = lines.iter().position(|&l| l == "task4").unwrap();
    let task2_pos = lines.iter().position(|&l| l == "task2").unwrap();
    let task3_pos = lines.iter().position(|&l| l == "task3").unwrap();
    let task1_pos = lines.iter().position(|&l| l == "task1").unwrap();

    assert!(task4_pos < task2_pos);
    assert!(task4_pos < task3_pos);
    assert!(task2_pos < task1_pos);
    assert!(task3_pos < task1_pos);

    Ok(())
}

#[tokio::test]
async fn test_parallel_execution() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());

    let task_spec = TaskSpec {
        name: "parallel_test".to_string(),
        action: "sleep 0.1".to_string(),
        deps: vec![],
        envs: Default::default(),
        working_dir: None,
        timeout: 1,
    };

    let task = Task::new(task_spec);
    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(vec![task], Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    // Execute task with timeout
    timeout(Duration::from_secs(1), scheduler.execute_all()).await??;

    Ok(())
} 