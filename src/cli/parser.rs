//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{value_parser, Arg, ArgMatches, Command};
use daggy::Dag;
use expanduser::expanduser;
use eyre::{eyre, Result};
use hex;
use sha2::{Digest, Sha256};
use glob;

use crate::cfg::config::{ConfigSpec, OttoSpec, ParamSpec, TaskSpec, TaskSpecs, Value};
use crate::cfg::env as env_eval;

pub type DAG<T> = Dag<T, (), u32>;

const OTTOFILES: &[&str] = &[
    "otto.yml",
    ".otto.yml",
    "otto.yaml",
    ".otto.yaml",
    "Ottofile",
    "OTTOFILE",
];

fn calculate_hash(action: &String) -> String {
    let mut hasher = Sha256::new();
    hasher.update(action);
    let result = hasher.finalize();
    hex::encode(&result)[..8].to_string()
}

fn ottofile_not_found_message() -> String {
    use colored::Colorize;

    let file_list = OTTOFILES.iter()
        .map(|f| format!("  {}", f.bright_yellow()))
        .collect::<Vec<_>>()
        .join("\n");

    format!("{}\n\nOtto looks for one of the following files:\n{}",
        "ERROR: No ottofile found in this directory or any parent directory!".red().bold(),
        file_list)
}

#[derive(Debug)]
pub struct OttofileNotFound;

impl std::fmt::Display for OttofileNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", ottofile_not_found_message())
    }
}

impl std::error::Error for OttofileNotFound {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub name: String,
    pub task_deps: Vec<String>,
    pub file_deps: Vec<String>,
    pub output_deps: Vec<String>,
    pub envs: HashMap<String, String>,
    pub values: HashMap<String, Value>,
    pub action: String,
    pub hash: String,
}

impl Task {
    #[must_use]
    pub fn new(
        name: String,
        task_deps: Vec<String>,
        file_deps: Vec<String>,
        output_deps: Vec<String>,
        envs: HashMap<String, String>,
        values: HashMap<String, Value>,
        action: String,
    ) -> Self {
        let hash = calculate_hash(&action);
        Self {
            name,
            task_deps,
            file_deps,
            output_deps,
            envs,
            values,
            action,
            hash,
        }
    }

    #[must_use]
    pub fn from_task_with_cwd_and_global_envs(task_spec: &TaskSpec, cwd: &std::path::Path, global_envs: &HashMap<String, String>) -> Self {
        let name = task_spec.name.clone();
        let task_deps = task_spec.before.clone();

        // Resolve file globs from input to canonical paths using explicit cwd
        let file_deps = Self::resolve_file_globs(&task_spec.input, cwd);

        // Resolve output globs to canonical paths using explicit cwd
        let output_deps = Self::resolve_file_globs(&task_spec.output, cwd);

        // Evaluate environment variables with two-level merging: global then task-level
        let evaluated_envs = Self::evaluate_merged_envs(global_envs, &task_spec.envs, cwd)
            .unwrap_or_else(|e| {
                eprintln!("Warning: Failed to evaluate environment variables for task '{}': {}", name, e);
                HashMap::new()
            });

        // Note: We do NOT add after tasks here since they depend on us, not vice versa
        // The after dependencies will be handled during DAG construction
        let values = HashMap::new();
        let action = task_spec.action.trim().to_string();  // Trim whitespace from script content
        Self::new(name, task_deps, file_deps, output_deps, evaluated_envs, values, action)
    }

    /// Evaluate and merge environment variables from global and task-level sources
    fn evaluate_merged_envs(
        global_envs: &HashMap<String, String>,
        task_envs: &HashMap<String, String>,
        cwd: &std::path::Path
    ) -> Result<HashMap<String, String>> {
        let mut merged_envs = HashMap::new();

        // First, evaluate and add global environment variables
        for (key, value) in global_envs {
            merged_envs.insert(key.clone(), value.clone());
        }

        // Then, evaluate and add task-level environment variables (overriding global ones)
        if !task_envs.is_empty() {
            let task_evaluated_envs = env_eval::evaluate_envs(task_envs, Some(cwd))?;
            for (key, value) in task_evaluated_envs {
                merged_envs.insert(key, value);
            }
        }

        Ok(merged_envs)
    }

    /// Resolve file globs to canonical paths
    fn resolve_file_globs(patterns: &[String], cwd: &std::path::Path) -> Vec<String> {
        let mut resolved_paths = Vec::new();

        for pattern in patterns {
            // Use glob to expand the pattern
            let full_pattern = if std::path::Path::new(pattern).is_absolute() {
                pattern.clone()
            } else {
                cwd.join(pattern).to_string_lossy().to_string()
            };

            match glob::glob(&full_pattern) {
                Ok(paths) => {
                    for path in paths {
                        match path {
                            Ok(p) => {
                                // Convert to canonical path
                                match fs::canonicalize(&p) {
                                    Ok(canonical) => resolved_paths.push(canonical.to_string_lossy().to_string()),
                                    Err(_) => {
                                        // If canonicalization fails, use the original path
                                        resolved_paths.push(p.to_string_lossy().to_string());
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Warning: Failed to resolve glob pattern '{}': {}", pattern, e);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Invalid glob pattern '{}': {}", pattern, e);
                    // If glob fails, treat as literal path
                    resolved_paths.push(pattern.clone());
                }
            }
        }

        resolved_paths
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Parser {
    prog: String,
    cwd: PathBuf,
    user: String,
    config_spec: ConfigSpec,
    hash: String,
    args: Vec<String>,
    pargs: Vec<Vec<String>>,
    ottofile: Option<PathBuf>,
}

impl Parser {
    pub fn new(args: Vec<String>) -> Result<Self> {
        let prog = args.get(0).cloned().unwrap_or_else(|| "otto".to_string());
        let cwd = env::current_dir()?;
        let user = env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        Ok(Self {
            prog,
            cwd,
            user,
            config_spec: ConfigSpec::default(),
            hash: String::new(),
            args,
            pargs: Vec::new(),
            ottofile: None,
        })
    }

    pub fn parse(&mut self) -> Result<(Vec<Task>, String, Option<PathBuf>)> {
        // Check for top-level help first, before any parsing
        if self.args.iter().any(|arg| arg == "--help" || arg == "-h") {
            // Load config for top-level help using Clap's argument parsing
            let default_otto_spec = OttoSpec::default();
            let otto_cmd = Self::otto_command(&default_otto_spec);

            let ottofile_value = match otto_cmd.try_get_matches_from(&self.args) {
                Ok(matches) => matches.get_one::<String>("ottofile")
                    .map(|s| s.clone())
                    .unwrap_or_else(|| "./".to_owned()),
                Err(_) => {
                    // Fall back to manual parsing if Clap fails
                    self.args.iter()
                        .position(|arg| arg == "-o" || arg == "--ottofile")
                        .and_then(|i| self.args.get(i + 1))
                        .cloned()
                        .unwrap_or_else(|| env::var("OTTOFILE").unwrap_or_else(|_| "./".to_owned()))
                }
            };

            let ottofile_path = Self::divine_ottofile(ottofile_value.clone());
            match ottofile_path {
                Ok(Some(path)) => {
                    let (mut config_spec, _hash, _ottofile) = Self::load_config_from_path(Some(path))?;

                    // Inject graph meta-task before showing help
                    Self::inject_graph_meta_task_into_tasks(&mut config_spec.tasks);

                    let mut help_cmd = Self::help_command(&config_spec.otto, &config_spec.tasks);
                    help_cmd.print_help()?;
                    std::process::exit(0);
                }
                Ok(None) | Err(_) => {
                    // No ottofile found, show help with error message
                    let default_otto_spec = OttoSpec::default();
                    let empty_tasks = TaskSpecs::new();
                    let mut help_cmd = Self::help_command(&default_otto_spec, &empty_tasks);
                    help_cmd.print_help()?;
                    std::process::exit(2);
                }
            }
        }

        // Check if help comes after a task name
        let mut help_after_task = false;
        let mut task_name = String::new();
        for i in 1..self.args.len() {
            if (self.args[i] == "--help" || self.args[i] == "-h") && i > 1 {
                // Check if previous arg is a task name (we'll need to load config first)
                let default_otto_spec = OttoSpec::default();
                let otto_cmd = Self::otto_command(&default_otto_spec);

                let ottofile_value = match otto_cmd.try_get_matches_from(&self.args) {
                    Ok(matches) => matches.get_one::<String>("ottofile")
                        .map(|s| s.clone())
                        .unwrap_or_else(|| "./".to_owned()),
                    Err(_) => {
                        // Fall back to manual parsing if Clap fails
                        self.args.iter()
                            .position(|arg| arg == "-o" || arg == "--ottofile")
                            .and_then(|i| self.args.get(i + 1))
                            .cloned()
                            .unwrap_or_else(|| env::var("OTTOFILE").unwrap_or_else(|_| "./".to_owned()))
                    }
                };

                let ottofile_path = Self::divine_ottofile(ottofile_value)?;
                let (config_spec, _hash, _ottofile) = Self::load_config_from_path(ottofile_path)?;

                task_name = self.args[i - 1].clone();
                // Check for both configured tasks and built-in meta-tasks
                if config_spec.tasks.contains_key(&task_name) || task_name == "graph" {
                    help_after_task = true;
                    self.config_spec = config_spec;
                    self.inject_graph_meta_task();
                    break;
                }
            }
        }

        if help_after_task {
            // Show task-specific help
            if let Some(task) = self.config_spec.tasks.get(&task_name) {
                let mut task_cmd = Self::task_to_command(task);
                task_cmd.print_help()?;
                std::process::exit(0);
            }
        }

        // Stage 1: Parse global options with default config
        let default_otto_spec = OttoSpec::default();
        let otto_cmd = Self::otto_command(&default_otto_spec);

        // Try to parse with allow_external_subcommands to capture remaining args
        let matches = match otto_cmd.try_get_matches_from(&self.args) {
            Ok(m) => m,
            Err(e) => {
                use clap::error::ErrorKind;
                match e.kind() {
                    ErrorKind::DisplayVersion | ErrorKind::DisplayHelp => {
                        e.print().expect("clap error print failed");
                        std::process::exit(0);
                    }
                    _ => return Err(eyre!(e)),
                }
            }
        };

        // Extract ottofile path and load config
        let ottofile_value = matches.get_one::<String>("ottofile")
            .map(|s| s.clone())
            .expect("ottofile should have a value from flag, env var, or default");

        let ottofile_path = Self::divine_ottofile(ottofile_value)?;
        let (config_spec, hash, ottofile) = Self::load_config_from_path(ottofile_path)?;

        self.config_spec = config_spec;
        self.hash = hash;
        self.ottofile = ottofile;

        // Inject built-in meta-tasks
        self.inject_graph_meta_task();

        // Stage 2: Extract remaining arguments manually from original args
        let remaining_args = self.extract_remaining_args(&matches)?;

        // Include both configured tasks AND built-in meta-tasks like "graph"
        let mut task_names: Vec<&str> = self.config_spec.tasks.keys().map(String::as_str).collect();
        task_names.push("graph"); // Always include graph as a built-in task name

        // Partition the remaining args by task names
        let partitions = partitions(&remaining_args, &task_names);
        self.pargs = partitions;

        // Extract task names from partitions
        let configured_tasks = self.pargs.iter()
            .filter_map(|p| if p.is_empty() { None } else { Some(p[0].clone()) })
            .collect::<Vec<String>>();

        // Process tasks and build DAG
        let tasks = self.process_tasks_with_filter(&configured_tasks)?;

        Ok((tasks, self.hash.clone(), self.ottofile.clone()))
    }

    pub fn parse_all_tasks(&mut self) -> Result<(Vec<Task>, String, Option<PathBuf>)> {
        // Load config if not already loaded
        if self.config_spec.tasks.is_empty() {
            // Parse command line arguments to extract ottofile path (similar to main parse method)
            let default_otto_spec = OttoSpec::default();
            let otto_cmd = Self::otto_command(&default_otto_spec);

            // Try to parse with allow_external_subcommands to capture ottofile flag
            let matches = match otto_cmd.try_get_matches_from(&self.args) {
                Ok(m) => m,
                Err(_) => {
                    // If parsing fails, fall back to default value
                    let ottofile_value = "./".to_owned();
                    let ottofile_path = Self::divine_ottofile(ottofile_value)?;
                    let (config_spec, hash, ottofile) = Self::load_config_from_path(ottofile_path)?;

                    self.config_spec = config_spec;
                    self.hash = hash;
                    self.ottofile = ottofile;

                    // Get all task names (excluding graph)
                    let all_task_names: Vec<String> = self.config_spec.tasks.keys()
                        .filter(|name| *name != "graph")
                        .cloned()
                        .collect();

                    // Process all tasks
                    let tasks = self.process_tasks_with_filter(&all_task_names)?;

                    return Ok((tasks, self.hash.clone(), self.ottofile.clone()));
                }
            };

            // Extract ottofile path from parsed arguments (Clap handles env var automatically)
            let ottofile_value = matches.get_one::<String>("ottofile")
                .map(|s| s.clone())
                .expect("ottofile should have a value from flag, env var, or default");

            let ottofile_path = Self::divine_ottofile(ottofile_value)?;
            let (config_spec, hash, ottofile) = Self::load_config_from_path(ottofile_path)?;

            self.config_spec = config_spec;
            self.hash = hash;
            self.ottofile = ottofile;
        }

        // Get all task names (excluding graph)
        let all_task_names: Vec<String> = self.config_spec.tasks.keys()
            .filter(|name| *name != "graph")
            .cloned()
            .collect();

        // Process all tasks
        let tasks = self.process_tasks_with_filter(&all_task_names)?;

        Ok((tasks, self.hash.clone(), self.ottofile.clone()))
    }

    fn extract_remaining_args(&self, _matches: &ArgMatches) -> Result<Vec<String>> {
        let mut remaining_args = Vec::new();
        let mut skip_next = false;
        let mut in_task_args = false;

        // Include both configured tasks AND built-in meta-tasks like "graph"
        let mut task_names: Vec<&str> = self.config_spec.tasks.keys().map(String::as_str).collect();
        task_names.push("graph"); // Always include graph as a built-in task name

        for (_i, arg) in self.args.iter().enumerate().skip(1) { // Skip program name
            if skip_next {
                skip_next = false;
                continue;
            }

            // Check if this is a global option that takes a value
            if arg == "-o" || arg == "--ottofile" ||
               arg == "-a" || arg == "--api" ||
               arg == "-j" || arg == "--jobs" ||
               arg == "-H" || arg == "--home" ||
               arg == "-t" || arg == "--tasks" ||
               arg == "-v" || arg == "--verbosity" ||
               arg == "-T" || arg == "--timeout" {
                skip_next = true; // Skip the value
                continue;
            }

            // Check if this is a global flag
            if arg == "-h" || arg == "--help" {
                continue; // Already handled
            }

            // Check if this is a task name
            if task_names.contains(&arg.as_str()) {
                in_task_args = true;
            }

            if in_task_args {
                remaining_args.push(arg.clone());
            }
        }

        Ok(remaining_args)
    }

    fn process_tasks_with_filter(&self, requested_tasks: &[String]) -> Result<Vec<Task>> {
        // Step 0: Evaluate global environment variables once
        let global_envs = if self.config_spec.otto.envs.is_empty() {
            HashMap::new()
        } else {
            env_eval::evaluate_envs(&self.config_spec.otto.envs, Some(&self.cwd))
                .unwrap_or_else(|e| {
                    eprintln!("Warning: Failed to evaluate global environment variables: {}", e);
                    HashMap::new()
                })
        };

        // Step 1: Compute all task dependencies using simple linear algorithm
        let task_deps = self.compute_task_deps()?;

        // Step 2: Find all tasks we need (requested + their transitive dependencies)
        let mut tasks_needed = HashSet::new();
        for task_name in requested_tasks {
            self.collect_transitive_deps(task_name, &task_deps, &mut tasks_needed)?;
        }

        // Step 3: Build task list from needed tasks
        let mut tasks = Vec::new();
        for task_name in &tasks_needed {
            let task_spec = self.config_spec.tasks.get(task_name)
                .ok_or_else(|| eyre!("Task '{}' not found", task_name))?;

            let mut task = Task::from_task_with_cwd_and_global_envs(task_spec, &self.cwd, &global_envs);

            // Find the partition for this task's arguments
            if let Some(task_args) = self.pargs.iter().find(|args| !args.is_empty() && args[0] == *task_name) {
                if task_args.len() > 1 {
                    // Parse task arguments using clap
                    let task_command = Self::task_to_command(task_spec);
                    let matches = task_command.get_matches_from(task_args);

                    for param_spec in task_spec.params.values() {
                        if let Some(value) = matches.get_one::<String>(param_spec.name.as_str()) {
                            task.values.insert(param_spec.name.clone(), Value::Item(value.to_string()));
                            // Also add to environment variables
                            task.envs.insert(param_spec.name.clone(), value.to_string());
                        }
                    }
                }
            }

            // Override task_deps with computed dependencies
            task.task_deps = task_deps.get(task_name)
                .map(|deps| deps.iter().cloned().collect())
                .unwrap_or_default();

            tasks.push(task);
        }

        Ok(tasks)
    }

    fn compute_task_deps(&self) -> Result<HashMap<String, Vec<String>>> {
        let mut task_deps: HashMap<String, Vec<String>> = HashMap::new();

        // Initialize with direct dependencies from 'before' field
        for (task_name, task_spec) in &self.config_spec.tasks {
            task_deps.insert(task_name.clone(), task_spec.before.clone());
        }

        // Add reverse dependencies from 'after' field
        for (task_name, task_spec) in &self.config_spec.tasks {
            for after_task in &task_spec.after {
                if let Some(deps) = task_deps.get_mut(after_task) {
                    if !deps.contains(task_name) {
                        deps.push(task_name.clone());
                    }
                }
            }
        }

        Ok(task_deps)
    }

    fn collect_transitive_deps(&self, task_name: &str, task_deps: &HashMap<String, Vec<String>>, collected: &mut HashSet<String>) -> Result<()> {
        if collected.contains(task_name) {
            return Ok(());
        }

        collected.insert(task_name.to_string());

        if let Some(deps) = task_deps.get(task_name) {
            for dep in deps {
                self.collect_transitive_deps(dep, task_deps, collected)?;
            }
        }

        Ok(())
    }

    fn otto_command(_otto_spec: &OttoSpec) -> Command {
        Command::new("otto")
            .version(env!("GIT_DESCRIBE"))
            .about("A task runner")
            .arg(
                Arg::new("ottofile")
                    .short('o')
                    .long("ottofile")
                    .value_name("PATH")
                    .help("path to the ottofile")
                    .default_value(".") // Use a static default
                    .env("OTTOFILE") // Tie to OTTOFILE environment variable
                    .value_parser(value_parser!(String))
            )
            .arg(
                Arg::new("api")
                    .short('a')
                    .long("api")
                    .value_name("URL")
                    .help("api url")
                    .default_value("http://localhost:8080") // Use a static default
                    .value_parser(value_parser!(String))
            )
            .arg(
                Arg::new("jobs")
                    .short('j')
                    .long("jobs")
                    .value_name("JOBS")
                    .help("number of jobs to run in parallel")
                    .default_value("1") // Use a static default
                    .value_parser(value_parser!(String))
            )
            .arg(
                Arg::new("home")
                    .short('H')
                    .long("home")
                    .value_name("PATH")
                    .help("home directory")
                    .default_value(".") // Use a static default
                    .value_parser(value_parser!(String))
            )
            .arg(
                Arg::new("tasks")
                    .short('t')
                    .long("tasks")
                    .value_name("TASKS")
                    .help("comma-separated list of tasks to run")
                    .value_parser(value_parser!(String))
            )
            .arg(
                Arg::new("verbosity")
                    .short('v')
                    .long("verbosity")
                    .value_name("LEVEL")
                    .help("verbosity level")
                    .default_value("0")
                    .value_parser(value_parser!(String))
            )
            .arg(
                Arg::new("timeout")
                    .short('T')
                    .long("timeout")
                    .value_name("SECONDS")
                    .help("timeout in seconds")
                    .default_value("0")
                    .value_parser(value_parser!(String))
            )
            .allow_external_subcommands(true)
    }

    fn task_to_command(task_spec: &TaskSpec) -> Command {
        let mut cmd = Command::new(task_spec.name.clone());

        if let Some(ref help) = task_spec.help {
            cmd = cmd.about(help.clone());
        }

        for param_spec in task_spec.params.values() {
            let arg = Self::param_to_arg(param_spec);
            cmd = cmd.arg(arg);
        }

        cmd
    }

    fn param_to_arg(param_spec: &ParamSpec) -> Arg {
        let mut arg = Arg::new(param_spec.name.clone());

        if let Some(short) = param_spec.short {
            arg = arg.short(short);
        }

        if let Some(ref long) = param_spec.long {
            arg = arg.long(long.clone());
        }

        if let Some(ref help) = param_spec.help {
            arg = arg.help(help.clone());
        }

        arg.value_parser(value_parser!(String))
    }

    fn help_command(_otto_spec: &OttoSpec, tasks: &TaskSpecs) -> Command {
        let mut cmd = Command::new("otto")
            .version(env!("GIT_DESCRIBE"))
            .about("A task runner")
            .arg(
                Arg::new("jobs")
                    .short('j')
                    .long("jobs")
                    .value_name("N")
                    .help("Number of parallel jobs")
                    .default_value("1")
                    .value_parser(value_parser!(String))
            )
            .arg(
                Arg::new("verbose")
                    .short('v')
                    .long("verbose")
                    .help("Verbose output")
                    .action(clap::ArgAction::SetTrue)
            )
            .arg(
                Arg::new("quiet")
                    .short('q')
                    .long("quiet")
                    .help("Quiet output")
                    .action(clap::ArgAction::SetTrue)
            )
            .arg(
                Arg::new("dry-run")
                    .long("dry-run")
                    .help("Show what would be done without executing")
                    .action(clap::ArgAction::SetTrue)
            )
            .arg(
                Arg::new("force")
                    .long("force")
                    .help("Force execution even if up-to-date")
                    .action(clap::ArgAction::SetTrue)
            )
            .arg(
                Arg::new("no-deps")
                    .long("no-deps")
                    .help("Don't run dependencies")
                    .action(clap::ArgAction::SetTrue)
            )
            .allow_external_subcommands(true);

        // Add tasks as subcommands
        if !tasks.is_empty() {
            // Collect all tasks except graph, sort them alphabetically
            let mut regular_tasks: Vec<_> = tasks.iter()
                .filter(|(name, _)| *name != "graph")
                .collect();
            regular_tasks.sort_by_key(|(name, _)| name.as_str());

            // Add regular tasks first
            for (_, task_spec) in regular_tasks {
                cmd = cmd.subcommand(Self::task_to_command(task_spec));
            }

            // Add graph task at the end if it exists
            if let Some(graph_task) = tasks.get("graph") {
                cmd = cmd.subcommand(Self::task_to_command(graph_task));
            }
        } else {
            cmd = cmd.after_help(ottofile_not_found_message());
        }

        cmd
    }

    fn inject_graph_meta_task(&mut self) {
        use crate::cfg::param::{ParamType, Nargs};

        // Add graph meta-task to the configuration
        let graph_task = TaskSpec {
            name: "graph".to_string(),
            help: Some("[built-in] Visualize the task dependency graph".to_string()),
            after: vec![],
            before: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: {
                let mut params = HashMap::new();

                // Add --format parameter
                params.insert("format".to_string(), ParamSpec {
                    name: "format".to_string(),
                    short: Some('f'),
                    long: Some("format".to_string()),
                    param_type: ParamType::OPT,
                    dest: None,
                    metavar: None,
                    default: Some("ascii".to_string()),
                    constant: Value::Empty,
                    choices: vec!["ascii".to_string(), "dot".to_string(), "svg".to_string(), "png".to_string(), "pdf".to_string()],
                    nargs: Nargs::One,
                    help: Some("Output format".to_string()),
                    value: Value::Empty,
                });

                // Add --output parameter
                params.insert("output".to_string(), ParamSpec {
                    name: "output".to_string(),
                    short: None,
                    long: Some("output".to_string()),
                    param_type: ParamType::OPT,
                    dest: None,
                    metavar: None,
                    default: None,
                    constant: Value::Empty,
                    choices: vec![],
                    nargs: Nargs::One,
                    help: Some("Output file path".to_string()),
                    value: Value::Empty,
                });

                params
            },
            action: "# Built-in graph command".to_string(),
        };

        self.config_spec.tasks.insert("graph".to_string(), graph_task);
    }

    fn inject_graph_meta_task_into_tasks(tasks: &mut TaskSpecs) {
        use crate::cfg::param::{ParamType, Nargs};

        // Add graph meta-task to the configuration
        let graph_task = TaskSpec {
            name: "graph".to_string(),
            help: Some("[built-in] Visualize the task dependency graph".to_string()),
            after: vec![],
            before: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: {
                let mut params = HashMap::new();

                // Add --format parameter
                params.insert("format".to_string(), ParamSpec {
                    name: "format".to_string(),
                    short: Some('f'),
                    long: Some("format".to_string()),
                    param_type: ParamType::OPT,
                    dest: None,
                    metavar: None,
                    default: Some("ascii".to_string()),
                    constant: Value::Empty,
                    choices: vec!["ascii".to_string(), "dot".to_string(), "svg".to_string(), "png".to_string(), "pdf".to_string()],
                    nargs: Nargs::One,
                    help: Some("Output format".to_string()),
                    value: Value::Empty,
                });

                // Add --output parameter
                params.insert("output".to_string(), ParamSpec {
                    name: "output".to_string(),
                    short: None,
                    long: Some("output".to_string()),
                    param_type: ParamType::OPT,
                    dest: None,
                    metavar: None,
                    default: None,
                    constant: Value::Empty,
                    choices: vec![],
                    nargs: Nargs::One,
                    help: Some("Output file path".to_string()),
                    value: Value::Empty,
                });

                params
            },
            action: "# Built-in graph command".to_string(),
        };

        tasks.insert("graph".to_string(), graph_task);
    }

    fn find_ottofile(path: &Path) -> Result<Option<PathBuf>> {
        for ottofile in OTTOFILES {
            let ottofile_path = path.join(ottofile);
            if ottofile_path.exists() {
                return Ok(Some(ottofile_path));
            }
        }
        // If we've reached the root, stop searching
        if let Some(parent) = path.parent() {
            if parent == path {
                return Ok(None);
            }
            // Recurse up
            Self::find_ottofile(parent)
        } else {
            Ok(None)
        }
    }

    fn divine_ottofile(value: String) -> Result<Option<PathBuf>> {
        let mut path = expanduser(value)?;
        path = fs::canonicalize(path)?;
        if path.is_dir() {
            return Self::find_ottofile(&path);
        }
        Ok(Some(path))
    }

    fn load_config_from_path(ottofile_path: Option<PathBuf>) -> Result<(ConfigSpec, String, Option<PathBuf>)> {
        if let Some(ottofile) = ottofile_path {
            let content = fs::read_to_string(&ottofile)?;
            let mut hasher = Sha256::new();
            hasher.update(&content);
            let result = hasher.finalize();
            let hash = hex::encode(&result)[..8].to_string();
            let config_spec: ConfigSpec = serde_yaml::from_str(&content)?;
            Ok((config_spec, hash, Some(ottofile)))
        } else {
            Err(eyre!("{}", ottofile_not_found_message()))
        }
    }
}

fn indices(args: &[String], task_names: &[&str]) -> Vec<usize> {
    let mut indices = vec![];
    for (i, arg) in args.iter().enumerate() {
        if task_names.contains(&arg.as_str()) {
            indices.push(i);
        }
    }
    indices
}

fn partitions(args: &Vec<String>, task_names: &[&str]) -> Vec<Vec<String>> {
    let task_indices = indices(args, task_names);
    if task_indices.is_empty() {
        return vec![];
    }

    let mut partitions = vec![];
    let mut end = args.len();

    for &index in task_indices.iter().rev() {
        partitions.insert(0, args[index..end].to_vec());
        end = index;
    }

    partitions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indices() {
        let args = vec!["task1".to_string(), "arg2".to_string(), "task2".to_string(), "arg3".to_string()];
        let task_names = &["task1", "task2"];
        let expected = vec![0, 2];
        assert_eq!(indices(&args, task_names), expected);
    }

    #[test]
    fn test_partitions() {
        let args = vec!["task1".to_string(), "arg2".to_string(), "task2".to_string(), "arg3".to_string()];
        let task_names = &["task1", "task2"];
        let expected = vec![
            vec!["task1".to_string(), "arg2".to_string()],
            vec!["task2".to_string(), "arg3".to_string()]
        ];
        assert_eq!(partitions(&args, task_names), expected);
    }

    #[test]
    fn test_partitions_empty() {
        let args = vec!["arg1".to_string(), "arg2".to_string()];
        let task_names = &["task1", "task2"];
        let expected: Vec<Vec<String>> = vec![];
        assert_eq!(partitions(&args, task_names), expected);
    }

    #[test]
    fn test_multiple_tasks_complex_args() {
        let args = vec![
            "build".to_string(),
            "--release".to_string(),
            "--target=x86_64-unknown-linux-gnu".to_string(),
            "test".to_string(),
            "--verbose".to_string(),
            "--filter=integration".to_string(),
            "deploy".to_string(),
            "--environment=staging".to_string(),
        ];

        let task_names = &["build", "test", "deploy"];
        let expected = vec![
            vec!["build".to_string(), "--release".to_string(), "--target=x86_64-unknown-linux-gnu".to_string()],
            vec!["test".to_string(), "--verbose".to_string(), "--filter=integration".to_string()],
            vec!["deploy".to_string(), "--environment=staging".to_string()]
        ];

        assert_eq!(partitions(&args, task_names), expected);
    }
}
