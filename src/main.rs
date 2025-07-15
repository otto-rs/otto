use std::env;
use std::sync::Arc;
use eyre::{Report, Result};
use log::info;
use env_logger::Target;
use std::fs::OpenOptions;
use otto::{
    cli::Parser,
    executor::{TaskScheduler, Workspace, DagVisualizer},
};

fn setup_logging() -> Result<(), Report> {
    let log_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre::eyre!("Could not determine local data directory"))?
        .join("otto")
        .join("logs");

    std::fs::create_dir_all(&log_dir)?;
    let log_file_path = log_dir.join("otto.log");

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)?;

    env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
        .target(Target::Pipe(Box::new(log_file)))
        .init();

    Ok(())
}

#[tokio::main]
async fn main() {
    // Setup logging first
    if let Err(e) = setup_logging() {
        eprintln!("Failed to setup logging: {}", e);
        std::process::exit(1);
    }
    info!("Starting otto");

    let args: Vec<String> = env::args().collect();

    // Create parser and parse arguments
    let mut parser = match Parser::new(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    let (tasks, hash, ottofile_path) = match parser.parse() {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    // Execute tasks
    if let Err(e) = execute_tasks(tasks, hash, ottofile_path).await {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

async fn execute_tasks(
    tasks: Vec<otto::cli::parser::Task>,
    _hash: String,
    _ottofile_path: Option<std::path::PathBuf>
) -> Result<(), Report> {
    if tasks.is_empty() {
        println!("No tasks to execute");
        return Ok(());
    }

    // Check if any task is a graph command
    let graph_tasks: Vec<_> = tasks.iter()
        .filter(|task| task.name == "graph")
        .collect();

    if !graph_tasks.is_empty() {
        // If there are graph tasks, handle them specially
        return DagVisualizer::execute_command(graph_tasks[0]).await;
    }

    // Filter out graph task for normal execution (shouldn't be needed now, but safety)
    let execution_tasks: Vec<_> = tasks.into_iter()
        .filter(|task| task.name != "graph")
        .collect();

    if execution_tasks.is_empty() {
        println!("No tasks to execute");
        return Ok(());
    }

    // Create workspace
    let cwd = env::current_dir()?;
    let workspace = Workspace::new(cwd).await?;
    workspace.init().await?;

    // Create execution context
    let execution_context = otto::executor::workspace::ExecutionContext::new();

    // Convert parser tasks to executor tasks
    let executor_tasks: Vec<otto::executor::Task> = execution_tasks.into_iter().map(|parser_task| {
        otto::executor::Task::new(
            parser_task.name,
            parser_task.task_deps,
            parser_task.file_deps,
            parser_task.output_deps,
            parser_task.envs,
            parser_task.values,
            parser_task.action,
        )
    }).collect();

    // Create task scheduler
    let jobs = num_cpus::get();
    let scheduler = TaskScheduler::new(
        executor_tasks,
        Arc::new(workspace),
        execution_context,
        jobs,
        jobs,
    ).await?;

    // Execute all tasks
    scheduler.execute_all().await?;

    Ok(())
}
