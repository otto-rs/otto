//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::env;
use std::collections::{HashMap, HashSet};
use eyre::{Report, eyre};
use log::info;
use env_logger::Target;
use std::fs::OpenOptions;
use std::sync::Arc;
use sha2::Digest;
use otto::{
    cli::{NomParser, ValidatedValue},
    executor::{TaskScheduler, Workspace, Task, DAG, graph::{DagVisualizer, GraphOptions, GraphFormat}},
    cfg::{config::{ConfigSpec, Value}, env as env_eval},
};


use eyre::Result;







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

    main_nom().await
}









async fn main_nom() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let command_line = args[1..].join(" ");

        // Extract ottofile path from command line arguments first
    let ottofile_path = {
        let args: Vec<&str> = command_line.split_whitespace().collect();
        let mut ottofile_value = None;

        // Look for --ottofile or -o arguments
        for i in 0..args.len() {
            if args[i] == "--ottofile" || args[i] == "-o" {
                if i + 1 < args.len() {
                    ottofile_value = Some(args[i + 1].to_string());
                    break;
                }
            } else if args[i].starts_with("--ottofile=") {
                ottofile_value = Some(args[i].split('=').nth(1).unwrap_or("").to_string());
                break;
            }
        }

        match ottofile_value {
            Some(path) => divine_ottofile(path)?,
            None => find_ottofile(&std::env::current_dir()?)?,
        }
    };

    // Check for help first (before parsing)
    if command_line.contains("--help") || command_line.contains("-h") {
        let args: Vec<&str> = command_line.split_whitespace().collect();

        // Check if help is the first argument (global help)
        if args.len() > 0 && (args[0] == "--help" || args[0] == "-h") {
            // Global help requested
            match load_config_from_path(ottofile_path.clone()) {
                Ok((config_spec, _, _)) => {
                    show_tasks_help(&config_spec);
                    return Ok(());
                }
                Err(_) => {
                    // No config found, show help with ottofile error details
                    show_help(true);
                    std::process::exit(2);
                }
            }
        }

        // Check if help comes after a task name
        for i in 1..args.len() {
            if (args[i] == "--help" || args[i] == "-h") && i > 0 {
                let potential_task_name = args[i - 1];

                // Try to load config to check if it's a valid task
                match load_config_from_path(ottofile_path.clone()) {
                    Ok((config_spec, _, _)) => {
                        if config_spec.tasks.contains_key(potential_task_name) {
                            // Show task-specific help
                            show_task_help(&config_spec, potential_task_name);
                            return Ok(());
                        }
                        // If not a valid task, continue with normal parsing
                        break;
                    }
                    Err(_) => {
                        // No config found, show help with ottofile error details
                        show_help(true);
                        std::process::exit(2);
                    }
                }
            }
        }
    }

    // Try to load config using the determined ottofile path
    let (config_spec, _hash, ottofile_path) = match load_config_from_path(ottofile_path) {
        Ok((config, hash, path)) => (Some(config), hash, path),
        Err(_e) => {
            // No config found - we'll handle this later based on what the user wants to do
            (None, String::new(), None)
        }
    };

    // Handle graph command before parsing (it's a built-in command)
    if command_line.trim() == "graph" || command_line.trim().starts_with("graph ") {
        // Check if help is requested for graph command
        if command_line.contains("--help") || command_line.contains("-h") {
            show_task_help(&ConfigSpec::default(), "graph");
            return Ok(());
        }

        if let Some(ref config) = config_spec {
            // Parse graph command arguments manually
            let mut arguments = HashMap::new();
            let parts: Vec<&str> = command_line.split_whitespace().collect();

            let mut i = 1; // Skip "graph"
            while i < parts.len() {
                match parts[i] {
                    "--format" | "-f" => {
                        if i + 1 < parts.len() {
                            arguments.insert("format".to_string(), otto::cli::types::ValidatedValue::String(parts[i + 1].to_string()));
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    "--output" => {
                        if i + 1 < parts.len() {
                            arguments.insert("output".to_string(), otto::cli::types::ValidatedValue::Path(std::path::PathBuf::from(parts[i + 1])));
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    _ => {
                        i += 1;
                    }
                }
            }

            let fake_parsed_task = otto::cli::types::ParsedTask {
                name: "graph".to_string(),
                arguments,
            };
            return handle_graph_command(config, &fake_parsed_task).await;
        } else {
            // No config found, show help with ottofile error details
            show_help(true);
            std::process::exit(2);
        }
    }

    // Create nom parser
    let mut parser = match NomParser::new(config_spec.clone()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    // Parse command line
    let parsed = match parser.parse(&command_line) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    // Handle help/version
    if parsed.global_options.help {
        if let Some(ref config) = config_spec {
            show_tasks_help(config);
        } else {
            show_help(true);
        }
        return Ok(());
    }

    if parsed.global_options.version {
        println!("{}", env!("GIT_DESCRIBE"));
        return Ok(());
    }

    // Need config for actual execution
    let config_spec = match config_spec {
        Some(config) => config,
        None => {
            // If no ottofile is found, always show helpful error message regardless of task specification
            show_help(true);
            std::process::exit(2);
        }
    };



    // Convert to execution format (reuse existing logic)
    let cwd = env::current_dir()?;
    let global_envs = if config_spec.otto.envs.is_empty() {
        HashMap::new()
    } else {
        env_eval::evaluate_envs(&config_spec.otto.envs, Some(&cwd))
            .unwrap_or_else(|e| {
                eprintln!("Warning: Failed to evaluate global environment variables: {}", e);
                HashMap::new()
            })
    };

    // Compute task dependencies
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
        if let Some(task_spec) = config_spec.tasks.get(task_name) {
            // Find the parsed task arguments if this task was explicitly requested
            let mut task_args = HashMap::new();
            for parsed_task in &parsed.tasks {
                if parsed_task.name == *task_name {
                    for (arg_name, validated_value) in &parsed_task.arguments {
                        let value = match validated_value {
                            ValidatedValue::String(s) => Value::Item(s.clone()),
                            ValidatedValue::Integer(i) => Value::Item(i.to_string()),
                            ValidatedValue::Float(f) => Value::Item(f.to_string()),
                            ValidatedValue::Boolean(b) => Value::Item(b.to_string()),
                            ValidatedValue::Path(p) => Value::Item(p.to_string_lossy().to_string()),
                            ValidatedValue::Url(u) => Value::Item(u.clone()),
                        };
                        task_args.insert(arg_name.clone(), value);
                    }
                    break;
                }
            }

            // Create task
            let task = Task {
                name: task_name.clone(),
                task_deps: task_deps.get(task_name).cloned().unwrap_or_default().into_iter().collect(),
                file_deps: task_spec.input.clone(),
                output_deps: task_spec.output.clone(),
                envs: {
                    let mut envs = global_envs.clone();
                    if let Ok(task_envs) = env_eval::evaluate_envs(&task_spec.envs, Some(&cwd)) {
                        envs.extend(task_envs);
                    }
                    envs
                },
                values: task_args,
                action: task_spec.action.clone(),
                hash: "".to_string(), // Will be computed later
            };

            tasks.push(task);
        }
    }

    // Setup workspace
    let workspace_root = if let Some(ref ottofile) = ottofile_path {
        ottofile.parent()
            .unwrap_or_else(|| ottofile.as_path())
            .to_path_buf()
    } else {
        env::current_dir()?
    };

    let workspace = Workspace::new(workspace_root).await?;
    workspace.init().await?;

    // Save execution context metadata
    let execution_context = ExecutionContext {
        prog: "otto".to_string(),
        cwd: cwd.clone(),
        user: env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
        timestamp: workspace.timestamp(),
        hash: workspace.hash().to_string(),
        ottofile: ottofile_path,
        args,
    };
    workspace.save_execution_context(execution_context.clone()).await?;

    // Execute tasks
    let scheduler = TaskScheduler::new(tasks, Arc::new(workspace), execution_context.clone(), config_spec.otto.jobs * 2, config_spec.otto.jobs).await?;
    scheduler.execute_all().await?;

    Ok(())
}

/// Show comprehensive help like the clap version
fn show_tasks_help(config_spec: &ConfigSpec) {
    let otto_spec = &config_spec.otto;

    println!("A task runner");
    println!();
    println!("\x1b[1mUsage:\x1b[0m \x1b[1motto\x1b[0m [OPTIONS] [COMMAND]");
    println!();

    // Show commands (tasks)
    if !config_spec.tasks.is_empty() {
        println!("\x1b[1mCommands:\x1b[0m");

        // Calculate the maximum command name length for proper alignment
        let mut max_command_len = "graph".len(); // Start with built-in commands
        max_command_len = max_command_len.max("help".len());

        // Check all task names
        for task_spec in config_spec.tasks.values() {
            max_command_len = max_command_len.max(task_spec.name.len());
        }

        // Add graph meta-task first
        println!("  \x1b[1m{:<width$}\x1b[0m  [built-in] Visualize the task dependency graph", "graph", width = max_command_len);

        // Sort tasks alphabetically
        let mut task_list: Vec<_> = config_spec.tasks.values().collect();
        task_list.sort_by(|a, b| a.name.cmp(&b.name));

        for task_spec in task_list {
            match &task_spec.help {
                Some(help) => println!("  \x1b[1m{:<width$}\x1b[0m  {}", task_spec.name, help, width = max_command_len),
                None => println!("  \x1b[1m{:<width$}\x1b[0m  {} task help", task_spec.name, task_spec.name, width = max_command_len),
            }
        }

        println!("  \x1b[1m{:<width$}\x1b[0m  Print this message or the help of the given subcommand(s)", "help", width = max_command_len);
        println!();
    }

    // Show options
    println!("\x1b[1mOptions:\x1b[0m");
    println!("  \x1b[1m-o, --ottofile <PATH>\x1b[0m    path to the ottofile [default: {}]", otto_spec.home);
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
fn divine_ottofile(value: String) -> Result<Option<std::path::PathBuf>, Report> {
    let mut path = expanduser::expanduser(value)?;
    path = std::fs::canonicalize(path)?;
    if path.is_dir() {
        return find_ottofile(&path);
    }
    Ok(Some(path))
}

// Helper function to load config
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

fn show_help(missing_ottofile: bool) {
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
    println!("  \x1b[1m-V, --version\x1b[0m            Print version");
    println!();
    println!("Logs are written to: {}", log_location);

    if missing_ottofile {
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
}

async fn handle_graph_command(config_spec: &ConfigSpec, parsed_task: &otto::cli::types::ParsedTask) -> Result<()> {
    let mut options = GraphOptions::default();

    // Default to ASCII format for terminal output
    options.format = GraphFormat::Ascii;

    // Parse arguments
    for (arg_name, validated_value) in &parsed_task.arguments {
        match arg_name.as_str() {
            "format" | "f" => {
                if let Some(ValidatedValue::String(s)) = Some(validated_value) {
                    options.format = match s.as_str() {
                        "ascii" => GraphFormat::Ascii,
                        "dot" => GraphFormat::Dot,
                        "svg" => GraphFormat::Svg,
                        "png" => GraphFormat::Png,
                        "pdf" => GraphFormat::Pdf,
                        _ => {
                            eprintln!("Warning: Unknown format '{}', using ASCII", s);
                            GraphFormat::Ascii
                        }
                    };
                }
            }
            "output" => {
                if let Some(ValidatedValue::Path(p)) = Some(validated_value) {
                    options.output_path = Some(p.to_path_buf());
                }
            }
            _ => {
                // Ignore unknown arguments
            }
        }
    }

    // Compute task dependencies
    let task_deps = compute_task_deps(&config_spec)?;

    // Get current working directory for environment evaluation
    let cwd = env::current_dir()?;
    let global_envs = if config_spec.otto.envs.is_empty() {
        HashMap::new()
    } else {
        env_eval::evaluate_envs(&config_spec.otto.envs, Some(&cwd))
            .unwrap_or_else(|e| {
                eprintln!("Warning: Failed to evaluate global environment variables: {}", e);
                HashMap::new()
            })
    };

    // Build tasks with computed dependencies
    let mut tasks = Vec::new();
    for task_name in config_spec.tasks.keys() {
        if let Some(task_spec) = config_spec.tasks.get(task_name) {
            let task = Task {
                name: task_name.clone(),
                task_deps: task_deps.get(task_name).cloned().unwrap_or_default().into_iter().collect(),
                file_deps: task_spec.input.clone(),
                output_deps: task_spec.output.clone(),
                envs: {
                    let mut envs = global_envs.clone();
                    if let Ok(task_envs) = env_eval::evaluate_envs(&task_spec.envs, Some(&cwd)) {
                        envs.extend(task_envs);
                    }
                    envs
                },
                values: HashMap::new(),
                action: task_spec.action.clone(),
                hash: "000000".to_string(), // Dummy hash for visualization
            };
            tasks.push(task);
        }
    }

    // Build DAG from tasks
    let dag = build_dag_from_tasks(&tasks)?;

    // Create visualizer and generate output
    let output_path = options.output_path.clone();
    let visualizer = DagVisualizer::new(options);
    let output = visualizer.visualize(&dag)?;

    // Output to file or stdout
    if let Some(output_path) = output_path {
        std::fs::write(&output_path, output)?;
        println!("Graph saved to {}", output_path.display());
    } else {
        println!("{}", output);
    }

    Ok(())
}

/// Build a DAG from a list of tasks
fn build_dag_from_tasks(tasks: &[Task]) -> Result<DAG<Task>> {
    let mut dag = DAG::new();
    let mut task_to_node = HashMap::new();

    // Add all tasks as nodes first
    for task in tasks {
        let node_index = dag.add_node(task.clone());
        task_to_node.insert(task.name.clone(), node_index);
    }

    // Add edges based on task dependencies
    for task in tasks {
        if let Some(target_node) = task_to_node.get(&task.name) {
            for dep_name in &task.task_deps {
                if let Some(source_node) = task_to_node.get(dep_name) {
                    // Add edge from dependency to task (dependency -> task)
                    dag.add_edge(*source_node, *target_node, ())?;
                }
            }
        }
    }

    Ok(dag)
}
