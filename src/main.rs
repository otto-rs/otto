//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::env;
use eyre::Report;
use std::sync::Arc;
use colored::Colorize;

use otto::{
    cli::parse::Parser,
    executor::{TaskScheduler, Workspace, graph::{DagVisualizer, GraphOptions, GraphFormat}},
};

#[tokio::main]
async fn main() -> Result<(), Report> {
    let args: Vec<String> = env::args().collect();
    let mut parser = Parser::new(args.clone())?;
    let parse_result = parser.parse();
    if let Err(e) = &parse_result {
        // Check for OttofileNotFound
        if let Some(_inner) = e.root_cause().downcast_ref::<otto::cli::parse::OttofileNotFound>() {
            // Print help with epilogue
            let epilogue = format!(
                "\n{}\n{}\n{}\n{}\n  - otto.yml\n  - .otto.yml\n  - otto.yaml\n  - .otto.yaml\n  - Ottofile\n  - OTTOFILE\n",
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
