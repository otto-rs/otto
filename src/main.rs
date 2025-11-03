use env_logger::Target;
use eyre::{Report, Result};
use log::info;
use otto::{
    cli::{CleanCommand, HistoryCommand, Parser, StatsCommand},
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

    // Check for built-in commands that don't require an ottofile
    if args.len() > 1 {
        match args[1].as_str() {
            "clean" => {
                if let Err(e) = execute_clean_command(&args[1..]).await {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
                return;
            }
            "history" => {
                if let Err(e) = execute_history_command(&args[1..]) {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
                return;
            }
            "stats" => {
                if let Err(e) = execute_stats_command(&args[1..]) {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
                return;
            }
            _ => {}
        }
    }

    // Create parser and parse arguments
    let mut parser = match Parser::new(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let (tasks, hash, ottofile_path, jobs, tui_mode) = match parser.parse() {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // Execute tasks
    if let Err(e) = execute_tasks(tasks, hash, ottofile_path, jobs, tui_mode).await {
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

    let clean_cmd = CleanCommand {
        keep_days,
        keep_last: None,
        keep_failed: None,
        dry_run,
        project_filter,
        no_db: false,
    };
    clean_cmd.execute().await?;

    Ok(())
}

fn execute_history_from_task(task: &otto::cli::parser::Task) -> Result<(), Report> {
    use otto::cfg::param::Value;
    use otto::cli::commands::history::HistoryCommand;

    // Extract task parameter
    let task_name = task
        .values
        .get("task")
        .and_then(|v| if let Value::Item(s) = v { Some(s.clone()) } else { None });

    // Extract limit parameter
    let limit = if let Some(Value::Item(s)) = task.values.get("limit") {
        s.parse::<usize>().unwrap_or(20)
    } else {
        20
    };

    // Extract status parameter
    let status = task
        .values
        .get("status")
        .and_then(|v| if let Value::Item(s) = v { Some(s.clone()) } else { None });

    // Extract project parameter
    let project = task
        .values
        .get("project")
        .and_then(|v| if let Value::Item(s) = v { Some(s.clone()) } else { None });

    // Extract json flag
    let json = task
        .values
        .get("json")
        .and_then(|v| if let Value::Item(s) = v { Some(s == "true") } else { None })
        .unwrap_or(false);

    let history_cmd = HistoryCommand {
        task_name,
        limit,
        status,
        project,
        json,
    };
    history_cmd.execute()?;

    Ok(())
}

fn execute_stats_from_task(task: &otto::cli::parser::Task) -> Result<(), Report> {
    use otto::cfg::param::Value;
    use otto::cli::commands::stats::StatsCommand;

    // Extract task parameter
    let task_name = task
        .values
        .get("task")
        .and_then(|v| if let Value::Item(s) = v { Some(s.clone()) } else { None });

    // Extract limit parameter
    let limit = if let Some(Value::Item(s)) = task.values.get("limit") {
        s.parse::<usize>().unwrap_or(10)
    } else {
        10
    };

    // Extract json flag
    let json = task
        .values
        .get("json")
        .and_then(|v| if let Value::Item(s) = v { Some(s == "true") } else { None })
        .unwrap_or(false);

    let stats_cmd = StatsCommand { task_name, limit, json };
    stats_cmd.execute()?;

    Ok(())
}

async fn execute_tasks(
    tasks: Vec<otto::cli::parser::Task>,
    hash: String,
    ottofile_path: Option<std::path::PathBuf>,
    jobs: usize,
    tui_mode: bool,
) -> Result<(), Report> {
    if tui_mode {
        // Check if we have a TTY
        if !atty::is(atty::Stream::Stdout) {
            eprintln!("Warning: --tui requires a TTY, falling back to standard output");
            return execute_with_terminal_output(tasks, hash, ottofile_path, jobs).await;
        }

        execute_with_tui(tasks, hash, ottofile_path, jobs).await
    } else {
        execute_with_terminal_output(tasks, hash, ottofile_path, jobs).await
    }
}

async fn execute_with_terminal_output(
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

    // Check if any task is a history command
    let history_tasks: Vec<_> = tasks.iter().filter(|task| task.name == "history").collect();
    if !history_tasks.is_empty() {
        return execute_history_from_task(history_tasks[0]);
    }

    // Check if any task is a stats command
    let stats_tasks: Vec<_> = tasks.iter().filter(|task| task.name == "stats").collect();
    if !stats_tasks.is_empty() {
        return execute_stats_from_task(stats_tasks[0]);
    }

    // Filter out built-in commands for normal execution
    let execution_tasks: Vec<_> = tasks
        .into_iter()
        .filter(|task| task.name != "graph" && task.name != "clean" && task.name != "history" && task.name != "stats")
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
    let scheduler = TaskScheduler::new(executor_tasks, Arc::new(workspace), execution_context, jobs, false).await?;

    // Execute all tasks
    scheduler.execute_all().await?;

    Ok(())
}

async fn execute_with_tui(
    tasks: Vec<otto::cli::parser::Task>,
    hash: String,
    ottofile_path: Option<std::path::PathBuf>,
    jobs: usize,
) -> Result<(), Report> {
    use otto::tui::{TaskPane, TuiApp};

    if tasks.is_empty() {
        eprintln!("No tasks to execute");
        return Ok(());
    }

    // Filter out built-in commands for normal execution
    let execution_tasks: Vec<_> = tasks
        .into_iter()
        .filter(|task| task.name != "graph" && task.name != "clean" && task.name != "history" && task.name != "stats")
        .collect();

    if execution_tasks.is_empty() {
        eprintln!("No tasks to execute");
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

    // Convert parser tasks to executor tasks and create TaskStreams
    let mut executor_tasks = Vec::new();
    let mut task_streams_map = std::collections::HashMap::new();
    let output_dir = workspace.run().join("tasks");

    for parser_task in execution_tasks {
        let task_name = parser_task.name.clone();

        // Create TaskStreams for this task
        let streams = otto::executor::output::TaskStreams::new(&task_name, &output_dir).await?;
        task_streams_map.insert(task_name.clone(), streams);

        // Create executor task
        let executor_task = otto::executor::Task::new(
            parser_task.name,
            parser_task.task_deps,
            parser_task.file_deps,
            parser_task.output_deps,
            parser_task.envs,
            parser_task.values,
            parser_task.action,
        );
        executor_tasks.push(executor_task);
    }

    // Initialize TUI
    let mut terminal = otto::tui::init_terminal().map_err(|e| eyre::eyre!("Failed to initialize TUI: {}", e))?;

    let mut app = TuiApp::new();

    // Create message broadcast channel for status updates (larger buffer for fast tasks)
    let (message_tx, _) = tokio::sync::broadcast::channel::<otto::executor::output::TaskMessage>(1000);

    // Create pane for each task
    for task in &executor_tasks {
        if let Some(streams) = task_streams_map.get(&task.name) {
            let mut pane = TaskPane::new(task.name.clone(), streams.output_tx.clone());
            pane.set_message_channel(message_tx.clone());
            app.layout_mut().add_pane(Box::new(pane));
        }
    }

    // Start scheduler in background with TUI mode enabled
    let mut scheduler = TaskScheduler::new(
        executor_tasks,
        Arc::new(workspace),
        execution_context,
        jobs,
        true, // tui_mode = true
    )
    .await?;

    // Set message channel on scheduler for broadcasting status updates
    scheduler.set_message_channel(message_tx);

    // Pass the pre-created task streams to the scheduler
    scheduler.set_task_streams(task_streams_map);

    // Draw initial TUI state before starting tasks (ensures receivers are ready)
    terminal.draw(|f| {
        app.layout_mut().render(f, f.area());
    })?;

    let scheduler_handle = tokio::spawn(async move { scheduler.execute_all().await });

    // Set up Ctrl+C handler flag
    let ctrl_c_pressed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ctrl_c_flag = ctrl_c_pressed.clone();

    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            ctrl_c_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });

    // Set the Ctrl+C flag in the app
    app.set_shutdown_flag(ctrl_c_pressed);

    // Run TUI (blocks until user quits or Ctrl+C)
    let tui_result = app.run(&mut terminal);

    // Always restore terminal, even on Ctrl+C or error
    if let Err(e) = otto::tui::restore_terminal(&mut terminal) {
        eprintln!("Warning: Failed to restore terminal: {}", e);
    }

    // Handle TUI errors
    tui_result.map_err(|e| eyre::eyre!("TUI error: {}", e))?;

    // Wait for scheduler to complete or propagate errors
    match scheduler_handle.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(eyre::eyre!("Scheduler panicked: {}", e)),
    }
}

async fn execute_clean_command(args: &[String]) -> Result<(), Report> {
    use clap::Parser;

    let clean_cmd = CleanCommand::parse_from(args);
    clean_cmd.execute().await?;
    Ok(())
}

fn execute_history_command(args: &[String]) -> Result<(), Report> {
    use clap::Parser;

    let history_cmd = HistoryCommand::parse_from(args);
    history_cmd.execute()?;
    Ok(())
}

fn execute_stats_command(args: &[String]) -> Result<(), Report> {
    use clap::Parser;

    let stats_cmd = StatsCommand::parse_from(args);
    stats_cmd.execute()?;
    Ok(())
}
