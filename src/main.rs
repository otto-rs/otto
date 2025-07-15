use std::env;
use std::sync::Arc;
use eyre::{Report, Result};
use log::info;
use env_logger::Target;
use std::fs::OpenOptions;
use otto::{
    cli::Parser,
    executor::{TaskScheduler, Workspace, DagVisualizer, GraphOptions, GraphFormat, NodeStyle},
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

    let args: Vec<String> = env::args().collect();

    // Create parser and parse arguments
    let mut parser = Parser::new(args)?;
    let (tasks, hash, ottofile_path) = parser.parse()?;

    // Execute tasks
    execute_tasks(tasks, hash, ottofile_path).await
}

async fn execute_tasks(
    tasks: Vec<otto::cli::parser::Task>,
    _hash: String,
    _ottofile_path: Option<std::path::PathBuf>
) -> Result<()> {
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
        return handle_graph_command(graph_tasks[0]).await;
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

async fn handle_graph_command(graph_task: &otto::cli::parser::Task) -> Result<()> {
    // Parse graph command arguments
    let format = graph_task.values.get("format")
        .and_then(|v| match v {
            otto::cfg::config::Value::Item(s) => Some(s.as_str()),
            _ => None,
        })
        .unwrap_or("ascii");

    let output_path = graph_task.values.get("output")
        .and_then(|v| match v {
            otto::cfg::config::Value::Item(s) => Some(std::path::PathBuf::from(s)),
            _ => None,
        });

    let graph_format = match format {
        "ascii" => GraphFormat::Ascii,
        "dot" => GraphFormat::Dot,
        "svg" => GraphFormat::Svg,
        "png" => GraphFormat::Png,
        "pdf" => GraphFormat::Pdf,
        _ => GraphFormat::Ascii,
    };

    let options = GraphOptions {
        show_details: true,
        show_file_deps: true,
        format: graph_format,
        style: NodeStyle::Detailed,
        output_path,
    };

    // We need to reload the parser to get all tasks for the graph
    let args: Vec<String> = env::args().collect();
    let mut parser = Parser::new(args)?;
    let (all_tasks, _, _) = parser.parse_all_tasks()?;

    // Convert parser tasks to executor tasks
    let executor_tasks: Vec<otto::executor::Task> = all_tasks.into_iter()
        .filter(|task| task.name != "graph")  // Exclude graph task itself
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
        }).collect();

    // Create DAG from tasks
    let dag = create_dag_from_tasks(executor_tasks)?;

    // Create visualizer and generate output
    let visualizer = DagVisualizer::new(options);
    let result = visualizer.visualize(&dag)?;

    println!("{}", result);

    Ok(())
}

fn create_dag_from_tasks(tasks: Vec<otto::executor::Task>) -> Result<otto::executor::DAG<otto::executor::Task>> {
    use daggy::Dag;
    use std::collections::HashMap;

    let mut dag: otto::executor::DAG<otto::executor::Task> = Dag::new();
    let mut task_indices = HashMap::new();

    // Add all tasks as nodes
    for task in tasks {
        let index = dag.add_node(task.clone());
        task_indices.insert(task.name.clone(), index);
    }

    // Add edges for dependencies
    // First, collect all the edges we need to add
    let mut edges_to_add = Vec::new();
    for (node_index, node_data) in dag.raw_nodes().iter().enumerate() {
        let task = &node_data.weight;
        let current_index = daggy::NodeIndex::new(node_index);

        for dep_name in &task.task_deps {
            if let Some(&dep_index) = task_indices.get(dep_name) {
                edges_to_add.push((dep_index, current_index, task.name.clone()));
            }
        }
    }

    // Now add all the edges
    for (dep_index, current_index, task_name) in edges_to_add {
        dag.add_edge(dep_index, current_index, ()).map_err(|e| {
            eyre::eyre!("Failed to add edge to {}: {:?}", task_name, e)
        })?;
    }

    Ok(dag)
}
