use env_logger::Target;
use eyre::{Report, Result};
use log::info;
use otto::{
    cli::{CleanCommand, HistoryCommand, Parser, StatsCommand},
    executor::{TaskScheduler, Workspace},
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
            "graph" => {
                if let Err(e) = execute_graph_command(&args[1..]).await {
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

async fn execute_clean_command(args: &[String]) -> Result<(), Report> {
    use clap::Parser;

    let clean_cmd = CleanCommand::parse_from(args);
    clean_cmd.execute().await?;
    Ok(())
}

async fn execute_graph_command(args: &[String]) -> Result<(), Report> {
    use otto::executor::{GraphFormat, GraphOptions, NodeStyle};

    // Parse basic arguments for graph command
    let mut format = GraphFormat::Ascii;
    let mut output_path = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--format" | "-f" => {
                if i + 1 < args.len() {
                    format = match args[i + 1].as_str() {
                        "dot" => GraphFormat::Dot,
                        "ascii" | "text" => GraphFormat::Ascii,
                        _ => GraphFormat::Ascii,
                    };
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--output" | "-o" => {
                if i + 1 < args.len() {
                    output_path = Some(std::path::PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }

    let options = GraphOptions {
        show_details: true,
        show_file_deps: true,
        format,
        style: NodeStyle::Detailed,
        output_path,
    };

    // Need to parse ottofile to generate graph
    let mut parser = otto::cli::Parser::new(vec!["otto".to_string()])?;
    let (all_tasks, _, _) = parser.parse_all_tasks()?;

    // Convert parser tasks to executor tasks and create DAG
    let dag = otto::executor::DagVisualizer::from_tasks(all_tasks)?;

    // Create visualizer with options
    let visualizer = otto::executor::DagVisualizer::new(options.clone());

    // Generate and display/save the graph
    let output = match options.format {
        GraphFormat::Ascii => visualizer.generate_ascii(&dag)?,
        GraphFormat::Dot => visualizer.generate_dot(&dag)?,
        _ => {
            return Err(eyre::eyre!("Unsupported format. Use 'ascii' or 'dot'."));
        }
    };

    if let Some(path) = &options.output_path {
        std::fs::write(path, &output)?;
        println!("Graph written to {}", path.display());
    } else {
        println!("{}", output);
    }

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

    // All tasks should be executed normally (built-in commands are handled earlier)
    let execution_tasks = tasks;

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
    let workspace_arc = Arc::new(workspace);
    let scheduler = TaskScheduler::new(executor_tasks, workspace_arc.clone(), execution_context, jobs).await?;

    // Execute all tasks
    let result = scheduler.execute_all().await;

    // Record run completion in database (graceful - doesn't fail if DB unavailable)
    workspace_arc.record_run_complete_in_db(result.is_ok());

    // Return the result
    result
}
