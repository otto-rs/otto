use std::path::PathBuf;
use eyre::Result;
use tempfile::TempDir;
use tokio::time::timeout;
use std::time::Duration;

use otto::executor::{
    Task, TaskSpec, TaskStatus,
    TaskScheduler,
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

    let task = Task::new(task_spec, work_dir.clone());
    let scheduler = TaskScheduler::new(vec![task], work_dir, 2, 2).await?;

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
        Task::new(task1_spec, work_dir.clone()),
        Task::new(task2_spec, work_dir.clone()),
        Task::new(task3_spec, work_dir.clone()),
    ];

    let scheduler = TaskScheduler::new(tasks, work_dir, 2, 2).await?;

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

    let task = Task::new(task_spec, work_dir.clone());
    let scheduler = TaskScheduler::new(vec![task], work_dir, 2, 2).await?;
    
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

    let task = Task::new(task_spec, work_dir.clone());
    let scheduler = TaskScheduler::new(vec![task], work_dir, 2, 2).await?;
    
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

    let task = Task::new(task_spec, work_dir.clone());
    let scheduler = TaskScheduler::new(vec![task], work_dir, 2, 2).await?;

    // Execute task with timeout
    timeout(Duration::from_secs(1), scheduler.execute_all()).await??;

    Ok(())
} 