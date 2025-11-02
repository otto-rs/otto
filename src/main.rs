use env_logger::Target;
use eyre::{Report, Result};
use log::info;
use otto::{
    cli::{CleanCommand, Parser},
    executor::{DagVisualizer, TaskScheduler, Workspace},
};
use std::env;
use std::fs::OpenOptions;
use std::sync::Arc;

fn setup_logging() -> Result<(), Report> {
    let log_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre::eyre!("Could not determine local data directory"))?
        .join("otto")
        .join("logs");

    std::fs::create_dir_all(&log_dir)?;
    let log_file_path = log_dir.join("otto.log");

    let log_file = OpenOptions::new().create(true).append(true).open(&log_file_path)?;

    env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
        .target(Target::Pipe(Box::new(log_file)))
        .init();

    Ok(())
}

#[tokio::main]
async fn main() {
    // Setup logging first
    if let Err(e) = setup_logging() {
        eprintln!("Failed to setup logging: {e}");
        std::process::exit(1);
    }
    info!("Starting otto");

    let args: Vec<String> = env::args().collect();

    // Create parser and parse arguments
    let mut parser = match Parser::new(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let (tasks, hash, ottofile_path, jobs) = match parser.parse() {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // Execute tasks
    if let Err(e) = execute_tasks(tasks, hash, ottofile_path, jobs).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

async fn execute_clean_from_task(task: &otto::cli::parser::Task) -> Result<(), Report> {
    use otto::cfg::param::Value;

    // Extract parameters from task values
    let keep_days = if let Some(Value::Item(s)) = task.values.get("keep") {
        s.parse::<u64>().unwrap_or(30)
    } else {
        30 // Default
    };

    // Boolean flag: stored as Value::Item("true") or Value::Item("false")
    let dry_run = task
        .values
        .get("dry-run")
        .and_then(|v| if let Value::Item(s) = v { Some(s == "true") } else { None })
        .unwrap_or(false);

    let project_filter = task
        .values
        .get("project")
        .and_then(|v| if let Value::Item(s) = v { Some(s.clone()) } else { None });

    let clean_cmd = CleanCommand::new(keep_days, dry_run, project_filter);
    clean_cmd.execute().await?;

    Ok(())
}

async fn execute_tasks(
    tasks: Vec<otto::cli::parser::Task>,
    hash: String,
    ottofile_path: Option<std::path::PathBuf>,
    jobs: usize,
) -> Result<(), Report> {
    if tasks.is_empty() {
        println!("No tasks to execute");
        return Ok(());
    }

    // Check if any task is a clean command
    let clean_tasks: Vec<_> = tasks.iter().filter(|task| task.name == "clean").collect();
    if !clean_tasks.is_empty() {
        return execute_clean_from_task(clean_tasks[0]).await;
    }

    // Check if any task is a graph command
    let graph_tasks: Vec<_> = tasks.iter().filter(|task| task.name == "graph").collect();
    if !graph_tasks.is_empty() {
        return DagVisualizer::execute_command(graph_tasks[0]).await;
    }

    // Filter out built-in commands for normal execution
    let execution_tasks: Vec<_> = tasks
        .into_iter()
        .filter(|task| task.name != "graph" && task.name != "clean")
        .collect();

    if execution_tasks.is_empty() {
        println!("No tasks to execute");
        return Ok(());
    }

    // Create workspace
    let cwd = env::current_dir()?;
    let workspace = Workspace::new(cwd).await?;
    workspace.init().await?;

    // Create execution context with ottofile path
    let mut execution_context = otto::executor::workspace::ExecutionContext::new();
    execution_context.ottofile = ottofile_path;
    execution_context.hash = hash;

    // Save execution context to run directory
    workspace.save_execution_context(execution_context.clone()).await?;

    // Convert parser tasks to executor tasks
    let executor_tasks: Vec<otto::executor::Task> = execution_tasks
        .into_iter()
        .map(|parser_task| {
            otto::executor::Task::new(
                parser_task.name,
                parser_task.task_deps,
                parser_task.file_deps,
                parser_task.output_deps,
                parser_task.envs,
                parser_task.values,
                parser_task.action,
            )
        })
        .collect();

    // Create task scheduler
    let scheduler = TaskScheduler::new(executor_tasks, Arc::new(workspace), execution_context, jobs).await?;

    // Execute all tasks
    scheduler.execute_all().await?;

    Ok(())
}
