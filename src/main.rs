//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::env;
use eyre::Report;
use std::sync::Arc;

use otto::{
    cli::parse::Parser,
    executor::{task::{Task, TaskSpec}, TaskScheduler, Workspace},
};

#[tokio::main]
async fn main() -> Result<(), Report> {
    let args: Vec<String> = env::args().collect();
    let mut parser = Parser::new(args.clone())?;

    let (otto, dag, _hash, ottofile_path) = parser.parse()?;

    // Use ottofile path for workspace hash calculation, fallback to current dir if not found
    let hash_path = if let Some(ottofile) = ottofile_path {
        ottofile
    } else {
        env::current_dir()?
    };

    let workspace = Workspace::new(hash_path).await?;
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

    // Convert DAG nodes into Tasks
    let mut tasks = Vec::new();
    let mut task_map = std::collections::HashMap::new();

    // First pass: Create tasks without dependencies
    for node in dag.raw_nodes() {
        let parse_spec = node.weight.clone();
        let task_spec = TaskSpec {
            name: parse_spec.name.clone(),
            action: parse_spec.action,
            deps: Vec::new(), // Start with empty deps, we'll fill them in second pass
            envs: parse_spec.envs,
            working_dir: None,
            timeout: otto.timeout.unwrap_or(0),
        };
        let task = Task::new(task_spec);
        task_map.insert(parse_spec.name, tasks.len());
        tasks.push(task);
    }

    // Second pass: Add dependencies from DAG edges
    for edge in dag.raw_edges() {
        let from = &dag.raw_nodes()[edge.source().index()].weight;
        let to = &dag.raw_nodes()[edge.target().index()].weight;
        if let Some(&to_idx) = task_map.get(&to.name) {
            tasks[to_idx].spec.deps.push(from.name.clone());
        }
    }

    let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), execution_context.clone(), otto.jobs * 2, otto.jobs).await?;
    scheduler.execute_all().await?;

    Ok(())
}
