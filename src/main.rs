use std::env;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use eyre::{Report, eyre, Result};
use log::info;
use env_logger::Target;
use std::fs::OpenOptions;
use std::sync::Arc;
use sha2::Digest;
use otto::{
    cli::{NomParser, ValidatedValue, parse_global_options_only},
    cli::validation::validate_global_options,
    executor::{TaskScheduler, Workspace, Task, DAG, graph::{DagVisualizer, GraphOptions, GraphFormat}},
    cfg::{config::{ConfigSpec, Value}},
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
    let command_line = args[1..].join(" ");

    main_two_pass(&command_line).await
}

async fn main_two_pass(command_line: &str) -> Result<()> {
    // ===== PASS 1: Parse global options only =====
    let (global_options, remaining_args) = match parse_global_options_only(command_line) {
        Ok((global_opts, remaining)) => {
            let validated_globals = validate_global_options(&global_opts)?;
            (validated_globals, remaining)
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(2);
        }
    };

    // Handle help/version early (short-circuit)
    if global_options.help {
        if remaining_args.trim().is_empty() {
            // For global help, check if ottofile exists to show appropriate error
            let ottofile_path = if let Some(ref ottofile) = global_options.ottofile {
                Some(ottofile.clone())
            } else {
                find_ottofile_in_current_and_parent_dirs()
            };

            let missing_ottofile = load_config_from_path(ottofile_path).is_err();
            show_help(missing_ottofile);
            if missing_ottofile {
                std::process::exit(2);
            }
        } else {
            // Parse the remaining args to get the task name for help
            let task_name = remaining_args.trim().split_whitespace().next().unwrap_or("");

            // Try to load config for task-specific help
            let ottofile_path = if let Some(ref ottofile) = global_options.ottofile {
                Some(ottofile.clone())
            } else {
                find_ottofile_in_current_and_parent_dirs()
            };

            if let Ok((config, _, _)) = load_config_from_path(ottofile_path) {
                // Use the existing HelpGenerator for task-specific help
                let help_output = otto::cli::help::HelpGenerator::generate_task_help(&config, task_name);
                println!("{}", help_output);
            } else {
                show_help(true);
            }
        }
        return Ok(());
    }

    if global_options.version {
        show_version();
        return Ok(());
    }

    // Handle built-in commands (like "graph")
    if remaining_args.trim() == "graph" || remaining_args.trim().starts_with("graph ") {
        // Determine ottofile path
        let ottofile_path = if let Some(ref ottofile) = global_options.ottofile {
            Some(ottofile.clone())
        } else {
            find_ottofile_in_current_and_parent_dirs()
        };

        // Try to load config
        let config = match load_config_from_path(ottofile_path) {
            Ok((config, _, _)) => config,
            Err(_) => {
                show_help(true);
                std::process::exit(2);
            }
        };

        // Parse graph command arguments
        let mut args = remaining_args.split_whitespace().skip(1); // Skip "graph"
        let mut format = GraphFormat::Ascii;
        let mut output_path = None;

        while let Some(arg) = args.next() {
            match arg {
                "--format" => {
                    if let Some(format_str) = args.next() {
                        format = match format_str {
                            "ascii" => GraphFormat::Ascii,
                            "dot" => GraphFormat::Dot,
                            "svg" => GraphFormat::Svg,
                            _ => return Err(eyre!("Invalid format: {}", format_str)),
                        };
                    }
                }
                "--output" => {
                    if let Some(path_str) = args.next() {
                        output_path = Some(PathBuf::from(path_str));
                    }
                }
                _ => {} // Ignore unknown arguments for now
            }
        }

        return handle_graph_command_two_pass(&config, format, output_path).await;
    }

    // ===== LOAD CONFIG (using ottofile from pass 1) =====
    let ottofile_path = if let Some(ref ottofile) = global_options.ottofile {
        Some(ottofile.clone())
    } else {
        find_ottofile_in_current_and_parent_dirs()
    };

    let (config_spec, _hash, ottofile_path) = match load_config_from_path(ottofile_path) {
        Ok((config, hash, path)) => (config, hash, path),
        Err(_) => {
            show_help(true);
            std::process::exit(2);
        }
    };

    // ===== PASS 2: Parse tasks with config-aware disambiguation =====
    let parser = NomParser::new(Some(config_spec.clone()))?;
    let tasks = match parser.parse_tasks_with_config(&remaining_args) {
        Ok(tasks) => tasks,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(2);
        }
    };

    // ===== EXECUTE =====
    execute_tasks(tasks, global_options, config_spec, ottofile_path).await
}

/// Helper function to find ottofile in current and parent directories
fn find_ottofile_in_current_and_parent_dirs() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    find_ottofile(&current_dir).ok().flatten()
}

/// Helper function to show version
fn show_version() {
    println!("otto {}", env!("GIT_DESCRIBE"));
}

/// Helper function to handle graph command with the new signature
async fn handle_graph_command_two_pass(config: &ConfigSpec, format: GraphFormat, output_path: Option<PathBuf>) -> Result<()> {
    // Build DAG from all tasks in config
    let tasks: Vec<Task> = config.tasks.iter().map(|(_name, task_spec)| {
        Task::from_task_with_cwd_and_global_envs(
            task_spec,
            Path::new("."),
            &std::collections::HashMap::new()
        )
    }).collect();

    let dag = build_dag_from_tasks(&tasks)?;

    // Create graph options
    let mut options = GraphOptions::default();
    options.format = format;
    options.output_path = output_path.clone();

    // Create visualizer and generate output
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

/// Helper function to execute tasks with the new signature
async fn execute_tasks(
    tasks: Vec<otto::cli::types::ParsedTask>,
    global_options: otto::cli::types::GlobalOptions,
    config_spec: ConfigSpec,
    ottofile_path: Option<PathBuf>,
) -> Result<()> {
    // Create workspace
    let workspace_root = ottofile_path.as_ref()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let workspace = Arc::new(Workspace::new(workspace_root).await?);
    workspace.init().await?;

    // Create execution context
    let execution_context = otto::executor::workspace::ExecutionContext {
        prog: "otto".to_string(),
        cwd: workspace.root().clone(),
        user: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        hash: workspace.hash().to_string(),
        ottofile: ottofile_path.clone(),
        args: std::env::args().collect(),
    };

    // Convert parsed tasks to Task objects
    let mut task_objects = Vec::new();
    for parsed_task in tasks {
        let task_spec = config_spec.tasks.get(&parsed_task.name).unwrap();

        // Convert ValidatedValue to Value for the task
        let mut values = HashMap::new();
        for (key, validated_value) in parsed_task.arguments {
            let value = match validated_value {
                ValidatedValue::String(s) => Value::Item(s),
                ValidatedValue::Integer(i) => Value::Item(i.to_string()),
                ValidatedValue::Float(f) => Value::Item(f.to_string()),
                ValidatedValue::Boolean(b) => Value::Item(b.to_string()),
                ValidatedValue::Path(p) => Value::Item(p.to_string_lossy().to_string()),
                ValidatedValue::Url(u) => Value::Item(u),
            };
            values.insert(key, value);
        }

        // Create task using the Task::from_task_with_cwd_and_global_envs method
        let mut task = Task::from_task_with_cwd_and_global_envs(
            task_spec,
            workspace.root(),
            &HashMap::new()
        );

        // Override values with CLI arguments
        task.values = values;

        task_objects.push(task);
    }

    // Create scheduler with proper parameters
    let io_limit = global_options.jobs.unwrap_or(4) as usize;
    let cpu_limit = global_options.jobs.unwrap_or(4) as usize;

    let scheduler = TaskScheduler::new(
        task_objects,
        workspace,
        execution_context,
        io_limit,
        cpu_limit,
    ).await?;

    // Execute tasks
    scheduler.execute_all().await
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
