//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::env;
use eyre::Report;
use std::sync::Arc;

use otto::{
    cli::parse::Parser,
    executor::{TaskScheduler, Workspace},
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
