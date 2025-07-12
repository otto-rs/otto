//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::env;
use eyre::Report;
use log::info;
use env_logger::Target;
use std::fs::OpenOptions;
#[cfg(feature = "nom-cli")]
use std::collections::HashSet;
#[cfg(feature = "nom-cli")]
use std::collections::HashMap;
#[cfg(feature = "nom-cli")]
use daggy::NodeIndex;
#[cfg(feature = "nom-cli")]
use eyre::eyre;
use eyre::Result;
#[cfg(feature = "nom-cli")]
use std::sync::Arc;
#[cfg(feature = "nom-cli")]
use sha2::Digest;

#[cfg(feature = "clap-cli")]
use colored::Colorize;

#[cfg(feature = "clap-cli")]
use std::sync::Arc;
#[cfg(feature = "clap-cli")]
use otto::{
    cli::parse::Parser,
    executor::{TaskScheduler, Workspace, graph::{DagVisualizer, GraphOptions, GraphFormat}},
};

#[cfg(feature = "nom-cli")]
use otto::{
    cli2::{NomParser, ValidatedValue},
    executor::{TaskScheduler, Workspace},
};

#[cfg(feature = "nom-cli")]
use otto::cli::parse::DAG;
#[cfg(feature = "nom-cli")]
use otto::cli::parse::Task;
#[cfg(feature = "nom-cli")]
use otto::cfg::config::ConfigSpec;
#[cfg(feature = "nom-cli")]
use otto::cfg::config::Value;
#[cfg(feature = "nom-cli")]
use otto::cfg::env as env_eval;
#[cfg(feature = "nom-cli")]
use otto::executor::workspace::ExecutionContext;

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
        return main_nom().await;
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
async fn main_nom() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    // First, extract ottofile path from arguments if present
    let ottofile_value = if let Some(pos) = args.iter().position(|arg| arg == "--ottofile" || arg == "-o") {
        args.get(pos + 1).cloned()
    } else {
        std::env::var("OTTOFILE").ok()
    };

    // Divine ottofile path - default to current directory if not specified (like clap)
    let ottofile_path = if let Some(value) = ottofile_value {
        divine_ottofile(value)?
    } else {
        // Default behavior: search current directory like clap does
        find_ottofile(&std::env::current_dir()?)?
    };

    // Handle help flag early, before loading config
    let input = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        String::new()
    };

    // Check for help flag - need to detect if it's task-specific or global
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        // Check if help comes after a task name (like clap does)
        let help_after_task = false;

        for i in 1..args.len() {
            if (args[i] == "--help" || args[i] == "-h") && i > 1 {
                // Check if previous arg is a task name (we'll need to load config first)
                let ottofile_value = args.iter()
                    .position(|arg| arg == "-o" || arg == "--ottofile")
                    .and_then(|i| args.get(i + 1))
                    .cloned()
                    .unwrap_or_else(|| std::env::var("OTTOFILE").unwrap_or_else(|_| "./".to_owned()));

                let ottofile_path = divine_ottofile(ottofile_value)?;

                // Try to load config, but handle missing ottofile gracefully
                match load_config_from_path(ottofile_path) {
                    Ok((config_spec, _hash, _ottofile)) => {
                        let task_name = args[i - 1].clone();
                        // Check for both configured tasks and built-in meta-tasks
                        if config_spec.tasks.contains_key(&task_name) || task_name == "graph" {
                            // Show task-specific help
                            show_task_help(&config_spec, &task_name);
                            std::process::exit(0);
                        }
                    }
                    Err(_) => {
                        // No ottofile found, show help with error message like clap
                        show_no_ottofile_help();
                        std::process::exit(2);
                    }
                }
            }
        }

        if !help_after_task {
            // Try to load config, but if it fails, show help with epilogue
            if let Ok((config_spec, _, _)) = load_config_from_path(ottofile_path.clone()) {
                // Show comprehensive help like clap version
                show_comprehensive_help(&config_spec);
                std::process::exit(0);
            } else {
                // No ottofile found, show help with error message like clap
                show_no_ottofile_help();
                std::process::exit(2);
            }
        }
    }

    // Quick check for version flag (doesn't need ottofile)
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("{}", env!("GIT_DESCRIBE"));
        std::process::exit(0);
    }

    // Load configuration
    let (config_spec, _hash, ottofile) = match load_config_from_path(ottofile_path) {
        Ok(result) => result,
        Err(_) => {
            // No ottofile found, show help with error message like clap
            show_no_ottofile_help();
            std::process::exit(2);
        }
    };
    let cwd = std::env::current_dir()?;

    // Parse command line arguments
    let mut parser = NomParser::new(Some(config_spec.clone()))?;
    let parsed = parser.parse(&input)?;

    // Handle help and version
    if parsed.global_options.help {
        // This should not happen anymore since we handle help above
        eprintln!("Unexpected help flag in parsed options");
        std::process::exit(0);
    }

    if parsed.global_options.version {
        println!("{}", env!("GIT_DESCRIBE"));
        std::process::exit(0);
    }

    // Evaluate global environment variables
    let global_envs = if config_spec.otto.envs.is_empty() {
        HashMap::new()
    } else {
        env_eval::evaluate_envs(&config_spec.otto.envs, Some(&cwd))
            .unwrap_or_else(|e| {
                eprintln!("Warning: Failed to evaluate global environment variables: {}", e);
                HashMap::new()
            })
    };

    // Compute task dependencies (handle both before and after fields)
    let task_deps = compute_task_deps(&config_spec)?;

    // Collect tasks to execute
    let mut requested_tasks = Vec::new();
    for parsed_task in &parsed.tasks {
        requested_tasks.push(parsed_task.name.clone());
    }

    // If no tasks specified, use defaults
    if requested_tasks.is_empty() {
        requested_tasks = config_spec.otto.tasks.clone();
    }

    // Find all tasks needed (requested + their transitive dependencies)
    let mut tasks_needed = HashSet::new();
    for task_name in &requested_tasks {
        collect_transitive_deps(task_name, &task_deps, &mut tasks_needed, &config_spec)?;
    }

    // Build tasks with computed dependencies
    let mut tasks = Vec::new();
    for task_name in &tasks_needed {
        let task_spec = config_spec.tasks.get(task_name)
            .ok_or_else(|| eyre!("Task '{}' not found in configuration", task_name))?;

        let mut task = Task::from_task_with_cwd_and_global_envs(task_spec, &cwd, &global_envs);

        // Override task_deps with computed dependencies
        task.task_deps = task_deps.get(task_name)
            .map(|deps| deps.iter().cloned().collect())
            .unwrap_or_default();

        // Apply parsed arguments to task if this was a requested task
        if let Some(parsed_task) = parsed.tasks.iter().find(|pt| pt.name == *task_name) {
            for (arg_name, validated_value) in &parsed_task.arguments {
                let value_str = match validated_value {
                    ValidatedValue::String(s) => s.clone(),
                    ValidatedValue::Boolean(b) => b.to_string(),
                    ValidatedValue::Integer(i) => i.to_string(),
                    ValidatedValue::Float(f) => f.to_string(),
                    ValidatedValue::Path(p) => p.to_string_lossy().to_string(),
                    ValidatedValue::Url(u) => u.clone(),
                };

                // Add to both envs and values
                task.envs.insert(arg_name.clone(), value_str.clone());
                task.values.insert(arg_name.clone(), Value::Item(value_str));
            }
        }

        tasks.push(task);
    }

    // Build DAG with proper dependencies
    let mut dag: DAG<Task> = DAG::new();
    let mut indices: HashMap<String, NodeIndex<u32>> = HashMap::new();

    // Add all tasks to DAG first
    for task in tasks {
        let task_name = task.name.clone();
        let index = dag.add_node(task);
        indices.insert(task_name, index);
    }

    // Add edges based on computed dependencies
    for task_name in &tasks_needed {
        let task_index = indices[task_name];
        if let Some(deps) = task_deps.get(task_name) {
            for dep_name in deps {
                if let Some(&dep_index) = indices.get(dep_name) {
                    dag.add_edge(dep_index, task_index, ())?;
                }
            }
        }
    }

    // Use ottofile directory for workspace hash calculation
    let workspace_root = if let Some(ref ottofile) = ottofile {
        ottofile.parent().unwrap_or_else(|| ottofile.as_path()).to_path_buf()
    } else {
        cwd.clone()
    };

    let workspace = Workspace::new(workspace_root).await?;
    workspace.init().await?;

    // Save execution context metadata
    let execution_context = ExecutionContext {
        prog: std::env::current_exe()?
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .map_or_else(|| "otto".to_string(), std::string::ToString::to_string),
        cwd: cwd.clone(),
        user: std::env::var("USER").unwrap_or_else(|_| "testuser".to_string()),
        timestamp: workspace.timestamp(),
        hash: workspace.hash().to_string(),
        ottofile: ottofile.clone(),
        args,
    };
    workspace.save_execution_context(execution_context.clone()).await?;

    // Convert DAG nodes into TaskSpecs for scheduler
    let mut task_specs = Vec::new();
    for node in dag.raw_nodes() {
        task_specs.push(node.weight.clone());
    }

    let scheduler = TaskScheduler::new(task_specs, Arc::new(workspace), execution_context.clone(), config_spec.otto.jobs * 2, config_spec.otto.jobs).await?;
    scheduler.execute_all().await?;

    Ok(())
}

/// Show comprehensive help like the clap version
#[cfg(feature = "nom-cli")]
fn show_comprehensive_help(config_spec: &ConfigSpec) {
    let otto_spec = &config_spec.otto;

    println!("A task runner");
    println!();
    println!("\x1b[1mUsage:\x1b[0m \x1b[1motto\x1b[0m [OPTIONS] [COMMAND]");
    println!();

    // Show commands (tasks)
    if !config_spec.tasks.is_empty() {
        println!("\x1b[1mCommands:\x1b[0m");

        // Add graph meta-task first
        println!("  \x1b[1mgraph\x1b[0m  [built-in] Visualize the task dependency graph");

        // Sort tasks alphabetically
        let mut task_list: Vec<_> = config_spec.tasks.values().collect();
        task_list.sort_by(|a, b| a.name.cmp(&b.name));

        for task_spec in task_list {
            match &task_spec.help {
                Some(help) => println!("  \x1b[1m{}\x1b[0m  {}", task_spec.name, help),
                None => println!("  \x1b[1m{}\x1b[0m  {} task help", task_spec.name, task_spec.name),
            }
        }

        println!("  \x1b[1mhelp\x1b[0m   Print this message or the help of the given subcommand(s)");
        println!();
    }

    // Show options
    println!("\x1b[1mOptions:\x1b[0m");
    println!("  \x1b[1m-o, --ottofile <PATH>\x1b[0m    path to the ottofile [default: ./]");
    println!("  \x1b[1m-a, --api <URL>\x1b[0m          api url [default: {}]", otto_spec.api);
    println!("  \x1b[1m-j, --jobs <JOBS>\x1b[0m        number of jobs to run in parallel [default: {}]", otto_spec.jobs);
    println!("  \x1b[1m-H, --home <PATH>\x1b[0m        path to the Otto home directory [default: {}]", otto_spec.home);

    // Show default tasks
    let default_tasks = if otto_spec.tasks.is_empty() {
        "*".to_string()
    } else {
        otto_spec.tasks.join(",")
    };
    println!("  \x1b[1m-t, --tasks <TASKS>\x1b[0m      comma separated list of tasks to run [default: {}]", default_tasks);

    println!("  \x1b[1m-v, --verbosity <LEVEL>\x1b[0m  verbosity level [default: {}]", otto_spec.verbosity);
    println!("  \x1b[1m-T, --timeout <SECONDS>\x1b[0m  global timeout in seconds (overrides task-specific timeouts)");
    println!("  \x1b[1m-V, --version\x1b[0m            Print version");
    println!();

    // Show log location
    let log_location = dirs::data_local_dir()
        .map(|dir| dir.join("otto").join("logs").join("otto.log"))
        .and_then(|path| path.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "~/.local/share/otto/logs/otto.log".to_string());

    println!("Logs are written to: {}", log_location);
}

/// Show task-specific help like clap does
#[cfg(feature = "nom-cli")]
fn show_task_help(config_spec: &ConfigSpec, task_name: &str) {
    if let Some(task_spec) = config_spec.tasks.get(task_name) {
        // Generate task-specific help similar to clap's task_to_command
        println!("{}", task_spec.help.as_deref().unwrap_or(&format!("{} task help", task_name)));
        println!();
        println!("\x1b[1mUsage:\x1b[0m \x1b[1m{}\x1b[0m [OPTIONS]", task_name);
        println!();

        if !task_spec.params.is_empty() {
            println!("\x1b[1mOptions:\x1b[0m");

            // Sort parameters for consistent output
            let mut params: Vec<_> = task_spec.params.iter().collect();
            params.sort_by_key(|(name, _)| name.as_str());

            for (param_name, param_spec) in params {
                let mut option_str = String::new();

                // Add short flag if available
                if let Some(short) = param_spec.short {
                    option_str.push_str(&format!("-{}, ", short));
                }

                // Add long flag
                if let Some(long) = &param_spec.long {
                    option_str.push_str(&format!("--{}", long));
                } else {
                    option_str.push_str(&format!("--{}", param_name));
                }

                // Add value placeholder
                option_str.push_str(&format!(" <{}>", param_name));

                // Build description
                let mut description = param_spec.help.clone().unwrap_or_else(|| format!("override {}", param_name));

                // Add default value
                if let Some(default) = &param_spec.default {
                    description.push_str(&format!(" [default: {}]", default));
                }

                println!("  \x1b[1m{}\x1b[0m  {}", option_str, description);
            }

            println!("  \x1b[1m-h, --help\x1b[0m                 Print help");
        } else {
            println!("\x1b[1mOptions:\x1b[0m");
            println!("  \x1b[1m-h, --help\x1b[0m                 Print help");
        }
    } else if task_name == "graph" {
        // Handle built-in graph task
        println!("[built-in] Visualize the task dependency graph");
        println!();
        println!("\x1b[1mUsage:\x1b[0m \x1b[1mgraph\x1b[0m [OPTIONS]");
        println!();
        println!("\x1b[1mOptions:\x1b[0m");
        println!("  \x1b[1m-f, --format <FORMAT>\x1b[0m  Output format: svg, png, pdf, dot, ascii [default: svg]");
        println!("  \x1b[1m--output <PATH>\x1b[0m        Output file path");
        println!("  \x1b[1m--tasks <TASKS>\x1b[0m        Filter to specific tasks");
        println!("  \x1b[1m-h, --help\x1b[0m             Print help");
    }
}

/// Compute task dependencies using simple linear-time algorithm
/// This mirrors the logic from src/cli/parse.rs
#[cfg(feature = "nom-cli")]
fn compute_task_deps(config_spec: &ConfigSpec) -> Result<HashMap<String, HashSet<String>>> {
    // Initialize empty dependency sets for all tasks
    let mut task_deps: HashMap<String, HashSet<String>> = HashMap::new();
    for task_name in config_spec.tasks.keys() {
        task_deps.insert(task_name.clone(), HashSet::new());
    }

    // Pass 1: Process 'before' edges - for each "before" edge, add u → t (u must precede t)
    for task_spec in config_spec.tasks.values() {
        for before_task in &task_spec.before {
            if !config_spec.tasks.contains_key(before_task) {
                return Err(eyre!("Task '{}' references unknown before dependency '{}'", task_spec.name, before_task));
            }
            task_deps.get_mut(&task_spec.name).unwrap().insert(before_task.clone());
        }
    }

    // Pass 2: Process 'after' edges - for each "after" edge, add t → v (t must precede v)
    // i.e. v depends on t
    for task_spec in config_spec.tasks.values() {
        for after_task in &task_spec.after {
            if !config_spec.tasks.contains_key(after_task) {
                return Err(eyre!("Task '{}' references unknown after dependency '{}'", task_spec.name, after_task));
            }
            task_deps.get_mut(after_task).unwrap().insert(task_spec.name.clone());
        }
    }

    Ok(task_deps)
}

/// Collect all transitive dependencies for a task
/// This mirrors the logic from src/cli/parse.rs
#[cfg(feature = "nom-cli")]
fn collect_transitive_deps(
    task_name: &str,
    task_deps: &HashMap<String, HashSet<String>>,
    collected: &mut HashSet<String>,
    config_spec: &ConfigSpec,
) -> Result<()> {
    if collected.contains(task_name) {
        return Ok(()); // Already processed
    }

    if !config_spec.tasks.contains_key(task_name) {
        return Err(eyre!("Task '{}' not found", task_name));
    }

    collected.insert(task_name.to_string());

    // Recursively collect dependencies
    if let Some(deps) = task_deps.get(task_name) {
        for dep in deps {
            collect_transitive_deps(dep, task_deps, collected, config_spec)?;
        }
    }

    Ok(())
}

// Helper function to find ottofile
#[cfg(feature = "nom-cli")]
fn find_ottofile(path: &std::path::Path) -> Result<Option<std::path::PathBuf>, Report> {
    const OTTOFILES: &[&str] = &[
        "otto.yml",
        ".otto.yml",
        "otto.yaml",
        ".otto.yaml",
        "Ottofile",
        "OTTOFILE",
    ];

    for ottofile in OTTOFILES {
        let ottofile_path = path.join(ottofile);
        if ottofile_path.exists() {
            return Ok(Some(ottofile_path));
        }
    }
    if let Some(parent) = path.parent() {
        if parent == path {
            return Ok(None);
        }
        find_ottofile(parent)
    } else {
        Ok(None)
    }
}

// Helper function to divine ottofile path
#[cfg(feature = "nom-cli")]
fn divine_ottofile(value: String) -> Result<Option<std::path::PathBuf>, Report> {
    let mut path = expanduser::expanduser(value)?;
    path = std::fs::canonicalize(path)?;
    if path.is_dir() {
        return find_ottofile(&path);
    }
    Ok(Some(path))
}

// Helper function to load config
#[cfg(feature = "nom-cli")]
fn load_config_from_path(ottofile_path: Option<std::path::PathBuf>) -> Result<(ConfigSpec, String, Option<std::path::PathBuf>), Report> {
    if let Some(ottofile) = ottofile_path {
        let content = std::fs::read_to_string(&ottofile)?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&content);
        let result = hasher.finalize();
        let hash = hex::encode(&result)[..8].to_string();
        let config_spec: ConfigSpec = serde_yaml::from_str(&content)?;
        Ok((config_spec, hash, Some(ottofile)))
    } else {
        Err(eyre!("No ottofile found in this directory or any parent directory!"))
    }
}

#[cfg(feature = "nom-cli")]
fn show_no_ottofile_help() {
    let log_location = dirs::data_local_dir()
        .map(|dir| dir.join("otto").join("logs").join("otto.log"))
        .and_then(|path| path.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "~/.local/share/otto/logs/otto.log".to_string());

    println!("A task runner");
    println!();
    println!("\x1b[1mUsage:\x1b[0m \x1b[1motto\x1b[0m [OPTIONS] [COMMAND]");
    println!();
    println!("\x1b[1mOptions:\x1b[0m");
    println!("  \x1b[1m-o, --ottofile <PATH>\x1b[0m    path to the ottofile [default: ./]");
    println!("  \x1b[1m-a, --api <URL>\x1b[0m          api url [default: 1]");
    println!("  \x1b[1m-j, --jobs <JOBS>\x1b[0m        number of jobs to run in parallel [default: 32]");
    println!("  \x1b[1m-H, --home <PATH>\x1b[0m        path to the Otto home directory [default: ~/.otto]");
    println!("  \x1b[1m-t, --tasks <TASKS>\x1b[0m      comma separated list of tasks to run [default: *]");
    println!("  \x1b[1m-v, --verbosity <LEVEL>\x1b[0m  verbosity level [default: 1]");
    println!("  \x1b[1m-T, --timeout <SECONDS>\x1b[0m  global timeout in seconds (overrides task-specific timeouts)");
    println!("  \x1b[1m-V, --version\x1b[0m            Print version");
    println!();
    println!("Logs are written to: {}", log_location);
    println!();
    println!("\x1b[1m\x1b[31mERROR: No ottofile found in this directory or any parent directory!\x1b[0m");
    println!("\x1b[33mOtto looks for one of the following files in the current or parent directories:\x1b[0m");
    println!();
    println!("To get started, create an otto.yml file in your project root.");
    println!("  - otto.yml");
    println!("  - .otto.yml");
    println!("  - otto.yaml");
    println!("  - .otto.yaml");
    println!("  - Ottofile");
    println!("  - OTTOFILE");
}
