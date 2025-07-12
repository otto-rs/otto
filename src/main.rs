//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::env;
use eyre::Report;
use log::info;
use env_logger::Target;
use std::fs::OpenOptions;

#[cfg(feature = "clap-cli")]
use std::sync::Arc;
#[cfg(feature = "clap-cli")]
use colored::Colorize;

#[cfg(feature = "clap-cli")]
use otto::{
    cli::parse::Parser,
    executor::{TaskScheduler, Workspace, graph::{DagVisualizer, GraphOptions, GraphFormat}},
};

#[cfg(feature = "nom-cli")]
use otto::{
    cli2::demo::run_demo,
    cli2::NomParser,
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
async fn main() -> Result<(), Report> {
    // Setup logging first
    setup_logging()?;
    info!("Starting otto");

    let _args: Vec<String> = env::args().collect();

    #[cfg(feature = "clap-cli")]
    {
        let mut parser = Parser::new(_args.clone())?;
        let parse_result = parser.parse();
        return main_clap(_args, parser, parse_result).await;
    }

    #[cfg(feature = "nom-cli")]
    {
        return main_nom(_args).await;
    }

    #[cfg(not(any(feature = "clap-cli", feature = "nom-cli")))]
    {
        eprintln!("Error: No CLI parser feature enabled. Enable either 'clap-cli' or 'nom-cli' feature.");
        std::process::exit(1);
    }
}

#[cfg(feature = "clap-cli")]
async fn main_clap(args: Vec<String>, parser: Parser, parse_result: Result<(otto::cfg::otto::OttoSpec, otto::cli::parse::DAG<otto::cli::parse::Task>, String, Option<std::path::PathBuf>), Report>) -> Result<(), Report> {
    if let Err(e) = &parse_result {
        // Check for OttofileNotFound
        if let Some(_inner) = e.root_cause().downcast_ref::<otto::cli::parse::OttofileNotFound>() {
            // Print help with epilogue
            let log_location = dirs::data_local_dir()
                .map(|dir| dir.join("otto").join("logs").join("otto.log"))
                .and_then(|path| path.to_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "~/.local/share/otto/logs/otto.log".to_string());

            let epilogue = format!(
                "Logs are written to: {}\n\n{}\n{}\n{}\n{}\n  - otto.yml\n  - .otto.yml\n  - otto.yaml\n  - .otto.yaml\n  - Ottofile\n  - OTTOFILE\n",
                log_location,
                "ERROR: No ottofile found in this directory or any parent directory!".bold().red(),
                "Otto looks for one of the following files in the current or parent directories:".yellow(),
                "",
                "To get started, create an otto.yml file in your project root."
            );
            let default_otto_spec = otto::cfg::config::OttoSpec::default();
            let mut help_cmd = otto::cli::parse::Parser::help_command(&default_otto_spec, &otto::cfg::config::ConfigSpec::default().tasks)
                .after_help(epilogue.clone())
                .after_long_help(epilogue);
            let _ = help_cmd.print_help();
            std::process::exit(2);
        } else {
            // Other errors
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
    let (otto, dag, _hash, ottofile_path) = parse_result?;

    // Check if graph visualization was requested
    if let Some(graph_result) = handle_graph_if_requested(&dag)? {
        println!("{}", graph_result);
        return Ok(());
    }

    // Use ottofile directory for workspace hash calculation, fallback to current dir if not found
    let workspace_root = if let Some(ottofile) = ottofile_path {
        // Use the directory containing the ottofile, not the ottofile itself
        ottofile.parent()
            .unwrap_or_else(|| ottofile.as_path())
            .to_path_buf()
    } else {
        // No ottofile found, use current directory
        env::current_dir()?
    };

    let workspace = Workspace::new(workspace_root).await?;
    workspace.init().await?;

    // Save execution context metadata
    let execution_context = otto::executor::workspace::ExecutionContext {
        prog: parser.prog().to_string(),
        cwd: parser.cwd().clone(),
        user: parser.user().to_string(),
        timestamp: workspace.timestamp(),
        hash: workspace.hash().to_string(),
        ottofile: parser.ottofile().map(|p| p.clone()),
        args,
    };
    workspace.save_execution_context(execution_context.clone()).await?;

    // Convert DAG nodes into TaskSpecs for scheduler
    let mut tasks = Vec::new();

    // Extract TaskSpecs from DAG nodes (they already have dependencies resolved)
    for node in dag.raw_nodes() {
        tasks.push(node.weight.clone());
    }

    let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), execution_context.clone(), otto.jobs * 2, otto.jobs).await?;
    scheduler.execute_all().await?;

    Ok(())
}

#[cfg(feature = "clap-cli")]
fn handle_graph_if_requested(dag: &otto::cli::parse::DAG<otto::cli::parse::Task>) -> Result<Option<String>, Report> {
    // Check if any task in the DAG is named "graph"
    let mut graph_task = None;
    let mut other_tasks = Vec::new();

    for node in dag.raw_nodes() {
        if node.weight.name == "graph" {
            graph_task = Some(&node.weight);
        } else {
            other_tasks.push(&node.weight);
        }
    }

    if let Some(graph_task) = graph_task {
        // Parse graph parameters from the graph task's parameter string
        let graph_options = parse_graph_parameters(graph_task)?;

        // Create a new DAG with only the tasks to visualize
        let viz_dag = if other_tasks.is_empty() {
            // If no other tasks specified, visualize all tasks except graph
            create_dag_without_graph_task(dag)?
        } else {
            // Create DAG with only the specified tasks and their dependencies
            create_dag_for_tasks_with_dependencies(dag, &other_tasks)?
        };

        let visualizer = DagVisualizer::new(graph_options);
        let result = visualizer.visualize(&viz_dag)?;
        return Ok(Some(result));
    }

    Ok(None)
}

#[cfg(feature = "clap-cli")]
fn parse_graph_parameters(task: &otto::cli::parse::Task) -> Result<GraphOptions, Report> {
    let mut options = GraphOptions::default();

    // Extract format from task values
    if let Some(format_value) = task.values.get("format") {
        if let otto::cfg::config::Value::Item(format_str) = format_value {
            options.format = match format_str.as_str() {
                "svg" => GraphFormat::Svg,
                "png" => GraphFormat::Png,
                "pdf" => GraphFormat::Pdf,
                "dot" => GraphFormat::Dot,
                "ascii" => GraphFormat::Ascii,
                _ => GraphFormat::Svg, // default
            };
        }
    }

    // Extract output path
    if let Some(output_value) = task.values.get("output") {
        if let otto::cfg::config::Value::Item(output_str) = output_value {
            options.output_path = Some(std::path::PathBuf::from(output_str));
        }
    }

    // Extract no-files flag
    if let Some(no_files_value) = task.values.get("no-files") {
        if let otto::cfg::config::Value::Item(no_files_str) = no_files_value {
            options.show_file_deps = no_files_str != "true";
        }
    }

    Ok(options)
}

#[cfg(feature = "clap-cli")]
fn create_dag_without_graph_task(original_dag: &otto::cli::parse::DAG<otto::cli::parse::Task>) -> Result<otto::cli::parse::DAG<otto::cli::parse::Task>, Report> {
    use otto::cli::parse::DAG;

    let mut new_dag = DAG::new();

    // Add all nodes except graph
    for node in original_dag.raw_nodes() {
        if node.weight.name != "graph" {
            new_dag.add_node(node.weight.clone());
        }
    }

    Ok(new_dag)
}

#[cfg(feature = "clap-cli")]
fn create_dag_for_tasks_with_dependencies(original_dag: &otto::cli::parse::DAG<otto::cli::parse::Task>, tasks: &[&otto::cli::parse::Task]) -> Result<otto::cli::parse::DAG<otto::cli::parse::Task>, Report> {
    use otto::cli::parse::DAG;

    let mut new_dag = DAG::new();

    // Add specified tasks and their dependencies
    let task_names: std::collections::HashSet<String> = tasks.iter().map(|t| t.name.clone()).collect();

    for node in original_dag.raw_nodes() {
        if task_names.contains(&node.weight.name) {
            new_dag.add_node(node.weight.clone());
        }
    }

    Ok(new_dag)
}

#[cfg(feature = "nom-cli")]
async fn main_nom(args: Vec<String>) -> Result<(), Report> {
    // For now, just run the demo to show that nom-cli is working
    // In a full implementation, this would:
    // 1. Parse command line arguments using NomParser
    // 2. Load configuration from ottofile
    // 3. Execute tasks like the clap version does

    println!("Otto CLI2 (nom-based) - Command: {:?}", args.get(1).unwrap_or(&"(none)".to_string()));

    // Check if this is a demo request
    if args.get(1).map(|s| s.as_str()) == Some("demo") {
        run_demo().await;
        return Ok(());
    }

    // For now, just show that we received the arguments
    if args.len() > 1 {
        println!("Received arguments: {:?}", &args[1..]);
        println!("This would be parsed by the nom-based CLI parser.");

        // Example of how the nom parser would be used:
        let input = args[1..].join(" ");
        println!("Input to parse: '{}'", input);

        // Create a basic parser (without config for now)
        let mut parser = NomParser::new(None).map_err(|e| eyre::eyre!("Failed to create parser: {}", e))?;

        match parser.parse(&input) {
            Ok(parsed) => {
                println!("✓ Parsed successfully:");
                println!("  Global options: {:?}", parsed.global_options);
                println!("  Tasks: {:?}", parsed.tasks);
            }
            Err(e) => {
                println!("✗ Parse error: {}", e);
            }
        }
    } else {
        println!("No arguments provided. Try 'otto demo' to see the nom-based parser demo.");
    }

    Ok(())
}
