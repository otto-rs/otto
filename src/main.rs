//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::{env, path::PathBuf};
use eyre::Report;

use otto::{
    cli::parse::Parser,
    executor::{task::{Task, TaskSpec}, TaskScheduler},
};

#[tokio::main]
async fn main() -> Result<(), Report> {
    let args: Vec<String> = env::args().collect();
    let mut parser = Parser::new(args)?;

    let (otto, dag, _) = parser.parse()?;
    let work_dir = PathBuf::from(&otto.home);

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

    let scheduler = TaskScheduler::new(tasks, work_dir, otto.jobs * 2, otto.jobs).await?;
    scheduler.execute_all().await?;

    Ok(())
}
