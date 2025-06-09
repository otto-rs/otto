use std::path::PathBuf;
use eyre::Result;
use tempfile::TempDir;
use tokio::time::timeout;
use std::time::Duration;
use std::sync::Arc;
use std::collections::HashMap;

use otto::executor::{
    Task, TaskStatus,
    TaskScheduler, TaskType, Workspace,
    workspace::ExecutionContext,
};

#[tokio::test]
async fn test_task_execution_with_output() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());

    let task_spec = Task::new(
        "test_task".to_string(),
        vec![],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        "echo hello".to_string(),
    );

    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(vec![task_spec], Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    // Execute task with timeout
    timeout(Duration::from_secs(5), scheduler.execute_all()).await??;

    Ok(())
}

#[tokio::test]
async fn test_task_dependencies() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());

    let task1_spec = Task::new(
        "task1".to_string(),
        vec![],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        "echo task1".to_string(),
    );

    let task2_spec = Task::new(
        "task2".to_string(),
        vec!["task1".to_string()],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        "echo task2".to_string(),
    );

    let task3_spec = Task::new(
        "task3".to_string(),
        vec!["task2".to_string()],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        "echo task3".to_string(),
    );

    let tasks = vec![task1_spec, task2_spec, task3_spec];

    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    // Execute task with timeout
    timeout(Duration::from_secs(5), scheduler.execute_all()).await??;

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

    let task_spec = Task::new(
        "failing_task".to_string(),
        vec![],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        "exit 1".to_string(),
    );

    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(vec![task_spec], Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    // Execute task with timeout - should fail
    let result = timeout(Duration::from_secs(5), scheduler.execute_all()).await?;
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_output_capture() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());

    let task_spec = Task::new(
        "output_test".to_string(),
        vec![],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        "echo 'hello world'".to_string(),
    );

    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(vec![task_spec], Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    // Execute task with timeout
    timeout(Duration::from_secs(5), scheduler.execute_all()).await??;

    Ok(())
}

#[tokio::test]
async fn test_dependency_ordering() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());
    let output_file = work_dir.join("output.txt");

    let task1_spec = Task::new(
        "task1".to_string(),
        vec!["task2".to_string(), "task3".to_string()],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        format!("echo 'task1' >> {}", output_file.display()),
    );

    let task2_spec = Task::new(
        "task2".to_string(),
        vec!["task4".to_string()],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        format!("echo 'task2' >> {}", output_file.display()),
    );

    let task3_spec = Task::new(
        "task3".to_string(),
        vec!["task4".to_string()],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        format!("echo 'task3' >> {}", output_file.display()),
    );

    let task4_spec = Task::new(
        "task4".to_string(),
        vec![],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        format!("echo 'task4' >> {}", output_file.display()),
    );

    let tasks = vec![task1_spec, task2_spec, task3_spec, task4_spec];

    let workspace = Workspace::new(work_dir.clone()).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    // Execute task with timeout
    timeout(Duration::from_secs(5), scheduler.execute_all()).await??;

    // Verify that task4 ran before task2 and task3, and they ran before task1
    let output = std::fs::read_to_string(output_file)?;
    let lines: Vec<&str> = output.lines().collect();

    let task4_pos = lines.iter().position(|&line| line == "task4").unwrap();
    let task2_pos = lines.iter().position(|&line| line == "task2").unwrap();
    let task3_pos = lines.iter().position(|&line| line == "task3").unwrap();
    let task1_pos = lines.iter().position(|&line| line == "task1").unwrap();

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

    let task1_spec = Task::new(
        "parallel1".to_string(),
        vec![],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        "sleep 0.5 && echo 'parallel1'".to_string(),
    );

    let task2_spec = Task::new(
        "parallel2".to_string(),
        vec![],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        "sleep 0.5 && echo 'parallel2'".to_string(),
    );

    let tasks = vec![task1_spec, task2_spec];

    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), 2, 2).await?;

    let start = std::time::Instant::now();
    // Execute task with timeout
    timeout(Duration::from_secs(10), scheduler.execute_all()).await??;
    let elapsed = start.elapsed();

    // Should take around 0.5 seconds due to parallel execution, not 1 second
    assert!(elapsed.as_secs_f32() < 0.8);

    Ok(())
}
