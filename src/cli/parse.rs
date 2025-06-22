//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{value_parser, Arg, ArgMatches, Command};
use daggy::{Dag, NodeIndex};
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

const DEFAULT_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn calculate_hash(action: &String) -> String {
    let mut hasher = Sha256::new();
    hasher.update(action);
    let result = hasher.finalize();
    hex::encode(&result)[..8].to_string()
}

#[derive(Debug)]
pub struct OttofileNotFound;

impl std::fmt::Display for OttofileNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No ottofile found in this directory or any parent directory!")
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
    pub fn from_task(task_spec: &TaskSpec) -> Self {
        let _name = task_spec.name.clone();
        let _task_deps = task_spec.before.clone();

        // Get current working directory for glob resolution
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        Self::from_task_with_cwd_and_global_envs(task_spec, &cwd, &HashMap::new())
    }

    #[must_use]
    pub fn from_task_with_cwd(task_spec: &TaskSpec, cwd: &std::path::Path) -> Self {
        Self::from_task_with_cwd_and_global_envs(task_spec, cwd, &HashMap::new())
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
    fn evaluate_merged_envs(global_envs: &HashMap<String, String>, task_envs: &HashMap<String, String>, working_dir: &std::path::Path) -> Result<HashMap<String, String>> {
        // Step 1: Create merged environment for task evaluation (global + task)
        let mut merged_envs = global_envs.clone();
        merged_envs.extend(task_envs.iter().map(|(k, v)| (k.clone(), v.clone())));

        // Step 2: Evaluate the merged environment (task envs can reference global envs)
        let evaluated_merged = if merged_envs.is_empty() {
            HashMap::new()
        } else {
            env_eval::evaluate_envs(&merged_envs, Some(working_dir))?
        };

        Ok(evaluated_merged)
    }

    /// Resolve file glob patterns to canonical file paths
    fn resolve_file_globs(patterns: &[String], cwd: &std::path::Path) -> Vec<String> {
        let mut resolved_files = Vec::new();

        for pattern in patterns {
            // Convert pattern to absolute path using provided cwd
            let pattern_path = if std::path::Path::new(pattern).is_absolute() {
                pattern.clone()
            } else {
                cwd.join(pattern).to_string_lossy().to_string()
            };

            // Use glob to expand patterns
            match glob::glob(&pattern_path) {
                Ok(paths) => {
                    let mut found_files = false;
                    for path in paths.flatten() {
                        found_files = true;
                        if let Ok(canonical) = path.canonicalize() {
                            resolved_files.push(canonical.to_string_lossy().to_string());
                        } else {
                            // If canonicalize fails, still add the path as-is
                            resolved_files.push(path.to_string_lossy().to_string());
                        }
                    }

                    // If glob succeeded but found no files, convert to absolute path anyway
                    if !found_files {
                        let abs_path = if std::path::Path::new(pattern).is_absolute() {
                            pattern.clone()
                        } else {
                            cwd.join(pattern).to_string_lossy().to_string()
                        };
                        resolved_files.push(abs_path);
                    }
                }
                Err(_) => {
                    // If glob fails, convert to absolute path anyway
                    let abs_path = if std::path::Path::new(pattern).is_absolute() {
                        pattern.clone()
                    } else {
                        cwd.join(pattern).to_string_lossy().to_string()
                    };
                    resolved_files.push(abs_path);
                }
            }
        }

        resolved_files
    }
}

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

impl Parser {
    pub fn new(args: Vec<String>) -> Result<Self> {
        let prog = std::env::current_exe()?
            .file_name()
            .and_then(OsStr::to_str)
            .map_or_else(|| "otto".to_string(), std::string::ToString::to_string);
        let cwd = env::current_dir()?;
        let user = env::var("USER").unwrap_or_else(|_| "testuser".to_string());

        // Initial empty config - we'll load it during parsing
        let config = ConfigSpec::default();
        let hash = DEFAULT_HASH.to_string();
        let ottofile = None;
        let pargs = vec![];

        Ok(Self {
            prog,
            cwd,
            user,
            config_spec: config,
            hash,
            args,
            pargs,
            ottofile,
        })
    }

    /// Get the program name
    pub fn prog(&self) -> &str {
        &self.prog
    }

    /// Get the current working directory when otto was run
    pub fn cwd(&self) -> &std::path::PathBuf {
        &self.cwd
    }

    /// Get the user who ran otto
    pub fn user(&self) -> &str {
        &self.user
    }

    /// Get the ottofile path if one was found
    pub fn ottofile(&self) -> Option<&std::path::PathBuf> {
        self.ottofile.as_ref()
    }

    fn find_ottofile(path: &Path) -> Result<Option<PathBuf>> {
        for ottofile in OTTOFILES {
            let ottofile_path = path.join(ottofile);
            if ottofile_path.exists() {
                // Return the absolute path directly instead of converting to relative
                return Ok(Some(ottofile_path));
            }
        }
        // If we've reached the root, stop searching
        if let Some(parent) = path.parent() {
            if parent == path {
                // We're at the root
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
            let hash = calculate_hash(&content);
            let config_spec: ConfigSpec = serde_yaml::from_str(&content)?;
            Ok((config_spec, hash, Some(ottofile)))
        } else {
            Err(eyre!(OttofileNotFound))
        }
    }

    /// Create the top-level Otto command with only global options (no subcommands)
    fn otto_command(otto_spec: &OttoSpec) -> Command {
        Command::new(&otto_spec.name)
            .bin_name(&otto_spec.name)
            .about(&otto_spec.about)
            .version(env!("GIT_DESCRIBE"))
            .arg(
                Arg::new("ottofile")
                    .short('o')
                    .long("ottofile")
                    .value_name("PATH")
                    .default_value("./")
                    .help("path to the ottofile"),
            )
            .arg(
                Arg::new("api")
                    .short('a')
                    .long("api")
                    .value_name("URL")
                    .default_value(&otto_spec.api)
                    .help("api url"),
            )
            .arg(
                Arg::new("jobs")
                    .short('j')
                    .long("jobs")
                    .value_name("JOBS")
                    .default_value(&otto_spec.jobs.to_string())
                    .value_parser(value_parser!(usize))
                    .help("number of jobs to run in parallel"),
            )
            .arg(
                Arg::new("home")
                    .short('H')
                    .long("home")
                    .value_name("PATH")
                    .default_value(&otto_spec.home)
                    .help("path to the Otto home directory"),
            )
            .arg(
                Arg::new("tasks")
                    .short('t')
                    .long("tasks")
                    .value_name("TASKS")
                    .default_values(&otto_spec.tasks)
                    .help("comma separated list of tasks to run"),
            )
            .arg(
                Arg::new("verbosity")
                    .short('v')
                    .long("verbosity")
                    .value_name("LEVEL")
                    .default_value("1")
                    .help("verbosity level"),
            )
            .arg(
                Arg::new("timeout")
                    .short('T')
                    .long("timeout")
                    .value_name("SECONDS")
                    .value_parser(value_parser!(u64))
                    .help("global timeout in seconds (overrides task-specific timeouts)"),
            )
            .disable_help_flag(true)  // We'll handle help manually
            .allow_external_subcommands(true)  // Allow unknown subcommands to pass through
    }

    /// Create the help command with all tasks as subcommands
    pub fn help_command(otto_spec: &OttoSpec, tasks: &TaskSpecs) -> Command {
        let mut command = Self::otto_command(otto_spec);

        // Sort tasks with built-in meta-tasks first, then alphabetically
        let mut task_list: Vec<_> = tasks.values().collect();
        task_list.sort_by(|a, b| {
            // Put built-in tasks (like graph) first
            match (a.name.as_str(), b.name.as_str()) {
                ("graph", _) => std::cmp::Ordering::Less,
                (_, "graph") => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        for task_spec in task_list {
            command = command.subcommand(Self::task_to_command(task_spec));
        }
        command
    }

    fn task_to_command(task_spec: &TaskSpec) -> Command {
        let mut command = Command::new(&task_spec.name).bin_name(&task_spec.name);
        if let Some(task_help) = &task_spec.help {
            command = command.about(task_help);
        }
        for param_spec in task_spec.params.values() {
            command = command.arg(Self::param_to_arg(param_spec));
        }
        command
    }

    fn param_to_arg(param_spec: &ParamSpec) -> Arg {
        let mut arg = Arg::new(&param_spec.name);
        if let Some(short) = param_spec.short {
            arg = arg.short(short);
        }
        if let Some(long) = &param_spec.long {
            arg = arg.long(long);
        }
        if let Some(help) = &param_spec.help {
            arg = arg.help(help);
        }
        if let Some(default) = &param_spec.default {
            arg = arg.default_value(default);
        }
        arg
    }

    pub fn parse(&mut self) -> Result<(OttoSpec, DAG<Task>, String, Option<PathBuf>)> {
        // Check for top-level help first, before any parsing
        if self.args.iter().any(|arg| arg == "--help" || arg == "-h") {
            // Check if help comes after a task name
            let mut help_after_task = false;
            let mut task_name = String::new();

            for i in 1..self.args.len() {
                if (self.args[i] == "--help" || self.args[i] == "-h") && i > 1 {
                    // Check if previous arg is a task name (we'll need to load config first)
                    let ottofile_value = self.args.iter()
                        .position(|arg| arg == "-o" || arg == "--ottofile")
                        .and_then(|i| self.args.get(i + 1))
                        .cloned()
                        .unwrap_or_else(|| env::var("OTTOFILE").unwrap_or_else(|_| "./".to_owned()));

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
            } else {
                // Load config for top-level help
                let ottofile_value = self.args.iter()
                    .position(|arg| arg == "-o" || arg == "--ottofile")
                    .and_then(|i| self.args.get(i + 1))
                    .cloned()
                    .unwrap_or_else(|| env::var("OTTOFILE").unwrap_or_else(|_| "./".to_owned()));

                let ottofile_path = Self::divine_ottofile(ottofile_value)?;
                let (config_spec, _hash, _ottofile) = Self::load_config_from_path(ottofile_path)?;

                let temp_config = config_spec;
                // Inject graph meta-task for help display
                let mut temp_parser = Parser {
                    prog: "otto".to_string(),
                    cwd: PathBuf::from("/"),
                    user: "temp".to_string(),
                    config_spec: temp_config,
                    hash: String::new(),
                    args: vec![],
                    pargs: vec![],
                    ottofile: None,
                };
                temp_parser.inject_graph_meta_task();

                let mut help_cmd = Self::help_command(&temp_parser.config_spec.otto, &temp_parser.config_spec.tasks);
                help_cmd.print_help()?;
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
            .unwrap_or_else(|| env::var("OTTOFILE").unwrap_or_else(|_| "./".to_owned()));

        let ottofile_path = Self::divine_ottofile(ottofile_value)?;
        let (config_spec, hash, ottofile) = Self::load_config_from_path(ottofile_path)?;

        // Update our internal state
        self.config_spec = config_spec;

        // Inject the graph meta-task into the configuration
        self.inject_graph_meta_task();

        self.hash = hash;
        self.ottofile = ottofile;

        // Stage 2: Extract remaining arguments manually from original args
        // We need to find where the otto options end and task args begin
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

        // Process Otto options from matches
        let mut otto = self.process_otto_options(matches)?;

        // Stage 3: Handle task arguments
        if remaining_args.is_empty() {
            // No tasks specified, use default tasks
            otto.tasks = self.config_spec.otto.tasks.clone();
            // Filter out tasks that don't exist in the configuration
            otto.tasks.retain(|task| self.config_spec.tasks.contains_key(task));
            if otto.tasks.is_empty() {
                // No tasks configured - show help instead of erroring
                let mut help_cmd = Self::help_command(&self.config_spec.otto, &self.config_spec.tasks);
                help_cmd.print_help()?;
                std::process::exit(0);
            }
        } else {
            // Check for task-level help
            if remaining_args.len() >= 2 && (remaining_args[1] == "-h" || remaining_args[1] == "--help") {
                let task_name = &remaining_args[0];
                if let Some(task_spec) = self.config_spec.tasks.get(task_name) {
                    let mut task_cmd = Self::task_to_command(task_spec);
                    task_cmd.print_help()?;
                    std::process::exit(0);
                } else {
                    return Err(eyre!("Task '{}' not found", task_name));
                }
            }

            // Partition the remaining args by task names
            let partitions = partitions(&remaining_args, &task_names);
            self.pargs = partitions;

            // Extract task names from partitions
            let configured_tasks = self.pargs.iter()
                .filter_map(|p| if p.is_empty() { None } else { Some(p[0].clone()) })
                .collect::<Vec<String>>();

            otto.tasks = configured_tasks;
        }

        // Process only the requested tasks and their dependencies
        let tasks = self.process_tasks_with_filter(&otto.tasks)?;

        Ok((otto, tasks, self.hash.clone(), self.ottofile.clone()))
    }

    fn process_otto_options(&self, matches: ArgMatches) -> Result<OttoSpec> {
        let mut otto = self.config_spec.otto.clone();

        if let Some(api) = matches.get_one::<String>("api") {
            otto.api = api.to_string();
        }
        if let Some(home) = matches.get_one::<String>("home") {
            otto.home = home.to_string();
        }
        if let Some(verbosity) = matches.get_one::<String>("verbosity") {
            otto.verbosity = verbosity.parse::<u8>().unwrap_or(1);
        }
        if let Some(jobs) = matches.get_one::<usize>("jobs") {
            otto.jobs = *jobs;
        }
        if let Some(timeout) = matches.get_one::<u64>("timeout") {
            otto.timeout = Some(*timeout);
        }

        Ok(otto)
    }

    fn process_tasks_with_filter(&self, requested_tasks: &[String]) -> Result<DAG<Task>> {
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

        // Step 3: Build DAG from needed tasks
        let mut dag: DAG<Task> = DAG::new();
        let mut indices: HashMap<String, NodeIndex<u32>> = HashMap::new();

        // Add all needed tasks to DAG first
        for task_name in &tasks_needed {
            let task_spec = self.config_spec.tasks.get(task_name)
                .ok_or_else(|| eyre!("Task '{}' not found", task_name))?;

            let mut spec = Task::from_task_with_cwd_and_global_envs(task_spec, &self.cwd, &global_envs);

            // Find the partition for this task's arguments
            if let Some(task_args) = self.pargs.iter().find(|args| !args.is_empty() && args[0] == *task_name) {
                if task_args.len() > 1 {
                    // Parse task arguments using clap
                    let task_command = Self::task_to_command(task_spec);
                    let matches = task_command.get_matches_from(task_args);

                    for param_spec in task_spec.params.values() {
                        if let Some(value) = matches.get_one::<String>(param_spec.name.as_str()) {
                            spec.values.insert(param_spec.name.clone(), Value::Item(value.to_string()));
                            // Also add to environment variables
                            spec.envs.insert(param_spec.name.clone(), value.to_string());
                        }
                    }
                }
            }

            // Override task_deps with computed dependencies
            spec.task_deps = task_deps.get(task_name)
                .map(|deps| deps.iter().cloned().collect())
                .unwrap_or_default();

            let index = dag.add_node(spec);
            indices.insert(task_name.clone(), index);
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

        Ok(dag)
    }

    /// Compute task dependencies using simple linear-time algorithm
    fn compute_task_deps(&self) -> Result<HashMap<String, HashSet<String>>> {
        // Initialize empty dependency sets for all tasks
        let mut task_deps: HashMap<String, HashSet<String>> = HashMap::new();
        for task_name in self.config_spec.tasks.keys() {
            task_deps.insert(task_name.clone(), HashSet::new());
        }

        // Pass 1: Process 'before' edges - for each "before" edge, add u → t (u must precede t)
        for task_spec in self.config_spec.tasks.values() {
            for before_task in &task_spec.before {
                if !self.config_spec.tasks.contains_key(before_task) {
                    return Err(eyre!("Task '{}' references unknown before dependency '{}'", task_spec.name, before_task));
                }
                task_deps.get_mut(&task_spec.name).unwrap().insert(before_task.clone());
            }
        }

        // Pass 2: Process 'after' edges - for each "after" edge, add t → v (t must precede v)
        // i.e. v depends on t
        for task_spec in self.config_spec.tasks.values() {
            for after_task in &task_spec.after {
                if !self.config_spec.tasks.contains_key(after_task) {
                    return Err(eyre!("Task '{}' references unknown after dependency '{}'", task_spec.name, after_task));
                }
                task_deps.get_mut(after_task).unwrap().insert(task_spec.name.clone());
            }
        }

        Ok(task_deps)
    }

    /// Collect all transitive dependencies for a task
    fn collect_transitive_deps(
        &self,
        task_name: &str,
        task_deps: &HashMap<String, HashSet<String>>,
        collected: &mut HashSet<String>,
    ) -> Result<()> {
        if collected.contains(task_name) {
            return Ok(()); // Already processed
        }

        if !self.config_spec.tasks.contains_key(task_name) {
            return Err(eyre!("Task '{}' not found", task_name));
        }

        collected.insert(task_name.to_string());

        // Recursively collect dependencies
        if let Some(deps) = task_deps.get(task_name) {
            for dep in deps {
                self.collect_transitive_deps(dep, task_deps, collected)?;
            }
        }

        Ok(())
    }

    /// Inject the graph meta-task into the configuration
    fn inject_graph_meta_task(&mut self) {
        use crate::cfg::config::{TaskSpec, ParamSpec};
        use crate::cfg::param::{ParamType, Value, Nargs};
        use std::collections::HashMap;

        // Don't inject if graph task already exists
        if self.config_spec.tasks.contains_key("graph") {
            return;
        }

        // Create parameters for the graph task
        let mut params = HashMap::new();

        // Add -f/--format parameter
        params.insert("format".to_string(), ParamSpec {
            name: "format".to_string(),
            short: Some('f'),
            long: Some("format".to_string()),
            param_type: ParamType::OPT,
            dest: None,
            metavar: None,
            default: Some("svg".to_string()),
            constant: Value::Empty,
            choices: vec![
                "svg".to_string(),
                "png".to_string(),
                "pdf".to_string(),
                "dot".to_string(),
                "ascii".to_string()
            ],
            nargs: Nargs::One,
            help: Some("Output format: svg, png, pdf, dot, ascii [default: svg]".to_string()),
            value: Value::Empty,
        });

        // Add --output parameter
        params.insert("output".to_string(), ParamSpec {
            name: "output".to_string(),
            short: None,
            long: Some("output".to_string()),
            param_type: ParamType::OPT,
            dest: None,
            metavar: Some("PATH".to_string()),
            default: None,
            constant: Value::Empty,
            choices: vec![],
            nargs: Nargs::One,
            help: Some("Output file path (auto-detected from extension if not specified)".to_string()),
            value: Value::Empty,
        });

        // Add --no-files flag
        params.insert("no-files".to_string(), ParamSpec {
            name: "no-files".to_string(),
            short: None,
            long: Some("no-files".to_string()),
            param_type: ParamType::FLG,
            dest: None,
            metavar: None,
            default: Some("false".to_string()),
            constant: Value::Empty,
            choices: vec![],
            nargs: Nargs::Zero,
            help: Some("Don't show file dependencies in the graph".to_string()),
            value: Value::Empty,
        });

        // Create the graph task spec
        let graph_task = TaskSpec {
            name: "graph".to_string(),
            help: Some("[built-in] Visualize the task dependency graph".to_string()),
            after: vec![],
            before: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params,
            action: "# Graph meta-task - parameters processed by Otto".to_string(),
            timeout: None,
        };

        // Insert the graph task into the configuration
        self.config_spec.tasks.insert("graph".to_string(), graph_task);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::collections::HashSet;
    use crate::cfg::param::{ParamType, Nargs};

    #[test]
    fn test_indices() {
        let args = vec_of_strings!["arg1", "task1", "arg2", "task2", "arg3",];
        let task_names = &["task1", "task2"];
        let expected = vec![1, 3];  // Only task indices, not including 0
        assert_eq!(indices(&args, task_names), expected);
    }

    #[test]
    fn test_partitions() {
        let args = vec_of_strings!["task1", "arg2", "task2", "arg3"];  // Start with task names
        let task_names = vec!["task1", "task2"];
        assert_eq!(
            partitions(&args, &task_names),
            vec![vec!["task1", "arg2"], vec!["task2", "arg3"]]
        );
    }

    #[test]
    fn test_task_dependencies() {
        let task_spec = TaskSpec {
            name: "A".to_string(),  // Changed from "main" to "A" for consistency
            action: "echo A".to_string(),
            before: vec!["dep1".to_string(), "before1".to_string()],
            after: vec!["after1".to_string()],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        };

        // Test that Task::from_task only includes before tasks as task_deps
        let spec = Task::from_task(&task_spec);
        let expected_task_deps: HashSet<String> = vec!["dep1".to_string(), "before1".to_string()]
            .into_iter()
            .collect();
        let actual_task_deps: HashSet<String> = spec.task_deps.into_iter().collect();
        assert_eq!(actual_task_deps, expected_task_deps, "Task should only include before tasks as task_deps");

        // Test DAG construction with before and after dependency types
        let mut tasks = HashMap::new();
        tasks.insert("A".to_string(), task_spec.clone());

        // Add the dependency tasks
        for name in ["dep1", "before1", "after1"] {
            let dep_task_spec = TaskSpec {
                name: name.to_string(),
                action: format!("echo {}", name),
                before: vec![],
                after: vec![],
                input: vec![],
                output: vec![],
                envs: HashMap::new(),
                params: HashMap::new(),
                help: None,
                timeout: Some(10),
            };
            tasks.insert(name.to_string(), dep_task_spec);
        }

        let args = vec!["otto".to_string()];
        let pargs = vec![args.clone()];  // Initialize pargs with just the program name

        let parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config_spec: ConfigSpec {
                otto: OttoSpec::default(),
                tasks: tasks.clone(),
            },
            hash: "test".to_string(),
            args,
            pargs,
            ottofile: None,
        };

        let dag = parser.process_tasks_with_filter(&[String::from("A")]).unwrap();  // Changed from "main" to "A"

        // Verify edges in the DAG
        let main_idx = (0..dag.raw_nodes().len())
            .map(NodeIndex::new)
            .find(|&i| dag[i].name == "A")  // Changed from "main" to "A"
            .expect("A task not found in DAG");
        let dep1_idx = (0..dag.raw_nodes().len())
            .map(NodeIndex::new)
            .find(|&i| dag[i].name == "dep1")
            .expect("dep1 task not found in DAG");
        let before1_idx = (0..dag.raw_nodes().len())
            .map(NodeIndex::new)
            .find(|&i| dag[i].name == "before1")
            .expect("before1 task not found in DAG");

        // Check that dep1 and before1 are dependencies of A
        assert!(dag.find_edge(dep1_idx, main_idx).is_some(), "dep1 should be a dependency of A");
        assert!(dag.find_edge(before1_idx, main_idx).is_some(), "before1 should be a dependency of A");

        // Verify DAG has the expected number of tasks (3: A + its dependencies)
        // Note: after1 is NOT included because it's not a dependency of A
        assert_eq!(dag.node_count(), 3, "DAG should contain A and its dependencies (dep1, before1)");

        // Test that after1 is NOT in the DAG (correct behavior)
        let after1_not_found = (0..dag.raw_nodes().len())
            .map(NodeIndex::new)
            .find(|&i| dag[i].name == "after1")
            .is_none();
        assert!(after1_not_found, "after1 should NOT be in DAG when only A is requested");

        // Now test requesting after1 directly to ensure after dependencies work
        let dag_with_after = parser.process_tasks_with_filter(&[String::from("after1")]).unwrap();

        // after1 depends on A (via A.after), which depends on dep1 and before1
        assert_eq!(dag_with_after.node_count(), 4, "When requesting after1, should get all 4 tasks");

        let after1_idx = (0..dag_with_after.raw_nodes().len())
            .map(NodeIndex::new)
            .find(|&i| dag_with_after[i].name == "after1")
            .expect("after1 should be in DAG when requested");
        let main_idx_after = (0..dag_with_after.raw_nodes().len())
            .map(NodeIndex::new)
            .find(|&i| dag_with_after[i].name == "A")  // Changed from "main" to "A"
            .expect("A should be in DAG as dependency of after1");

        // Check that A is a dependency of after1 (A runs before after1)
        assert!(dag_with_after.find_edge(main_idx_after, after1_idx).is_some(), "A should be a dependency of after1");

        // Verify edges: A -> B and C -> B
        assert!(dag.find_edge(dep1_idx, main_idx).is_some(), "A should run before B");
        assert!(dag.find_edge(before1_idx, main_idx).is_some(), "C should run before B");

        // Test requesting just A (should get A and its dependencies)
        let dag_a = parser.process_tasks_with_filter(&[String::from("A")]).unwrap();
        assert_eq!(dag_a.node_count(), 3, "DAG should contain A and its dependencies when requesting A");
    }

    #[test]
    fn test_task_selection() -> Result<()> {
        // Test the partitioning logic for task selection
        let args = vec!["standalone".to_string()];
        let task_names = vec!["standalone"];
        let result = partitions(&args, &task_names);

        assert_eq!(result, vec![vec!["standalone"]]);

        // Test dependent task
        let args = vec!["dependent".to_string()];
        let task_names = vec!["dependent"];
        let result = partitions(&args, &task_names);

        assert_eq!(result, vec![vec!["dependent"]]);

        Ok(())
    }

    #[test]
    fn test_parameter_passing() -> Result<()> {
        // Test parameter parsing logic
        let args = vec!["greet".to_string(), "-g".to_string(), "howdy".to_string()];
        let task_names = vec!["greet"];
        let result = partitions(&args, &task_names);

        assert_eq!(result, vec![vec!["greet", "-g", "howdy"]]);

        Ok(())
    }

    #[test]
    fn test_multi_task_parsing() -> Result<()> {
        // This test simulates the actual parsing flow without pre-loading config
        // We'll test the partitioning logic separately

        // Test the partitioning logic directly
        let args = vec!["hello".to_string(), "-g".to_string(), "howdy".to_string(), "world".to_string(), "-n".to_string(), "mundo".to_string()];
        let task_names = vec!["hello", "world"];
        let result = partitions(&args, &task_names);

        assert_eq!(result, vec![
            vec!["hello", "-g", "howdy"],
            vec!["world", "-n", "mundo"]
        ]);

        Ok(())
    }

    #[test]
    fn test_global_options_with_tasks() -> Result<()> {
        // Test that we can identify global options vs task arguments
        let args = vec!["otto".to_string(), "-j".to_string(), "4".to_string(), "test".to_string()];

        // Test that we can extract the task name from the args
        let task_names = vec!["test"];
        let mut remaining_args = Vec::new();
        let mut found_task = false;

        for arg in &args[1..] { // Skip program name
            if task_names.contains(&arg.as_str()) {
                found_task = true;
            }
            if found_task {
                remaining_args.push(arg.clone());
            }
        }

        assert_eq!(remaining_args, vec!["test"]);
        Ok(())
    }

    #[test]
    fn test_partitions_with_multi_tasks() {
        let args = vec_of_strings!["hello", "-g", "howdy", "world", "-n", "mundo"];
        let task_names = vec!["hello", "world"];
        let result = partitions(&args, &task_names);

        assert_eq!(result, vec![
            vec!["hello", "-g", "howdy"],
            vec!["world", "-n", "mundo"]
        ]);
    }

    #[test]
    fn test_single_task_with_multiple_params() {
        let args = vec_of_strings!["hello", "-g", "howdy", "--verbose", "true"];
        let task_names = vec!["hello"];
        let result = partitions(&args, &task_names);

        assert_eq!(result, vec![
            vec!["hello", "-g", "howdy", "--verbose", "true"]
        ]);
    }

    #[test]
    fn test_help_argument_detection() {
        // Test detection of help flags
        let help_args = vec![
            vec!["otto".to_string(), "--help".to_string()],
            vec!["otto".to_string(), "-h".to_string()],
            vec!["otto".to_string(), "task1".to_string(), "--help".to_string()],
            vec!["otto".to_string(), "task1".to_string(), "-h".to_string()],
        ];

        for args in help_args {
            let has_help = args.iter().any(|arg| arg == "--help" || arg == "-h");
            assert!(has_help, "Should detect help flag in args: {:?}", args);
        }
    }

    #[test]
    fn test_global_options_extraction() {
        // Test that we can identify global options vs task arguments
        let args = vec!["otto".to_string(), "-j".to_string(), "4".to_string(), "-v".to_string(), "2".to_string(), "hello".to_string(), "-g".to_string(), "test".to_string()];

        let global_options = vec!["-j", "-v", "-o", "-a", "-H", "-t", "-T"];
        let task_names = vec!["hello"];

        // Simulate the extraction logic
        let mut remaining_args = Vec::new();
        let mut skip_next = false;
        let mut in_task_args = false;

        for arg in &args[1..] { // Skip program name
            if skip_next {
                skip_next = false;
                continue;
            }

            // Check if this is a global option that takes a value
            if global_options.iter().any(|&opt| arg == opt || arg == &format!("--{}", opt.trim_start_matches('-'))) {
                skip_next = true; // Skip the value
                continue;
            }

            // Check if this is a task name
            if task_names.contains(&arg.as_str()) {
                in_task_args = true;
            }

            if in_task_args {
                remaining_args.push(arg.clone());
            }
        }

        assert_eq!(remaining_args, vec!["hello", "-g", "test"]);
    }

    #[test]
    fn test_multiple_tasks_complex_args() {
        // Test complex multi-task scenarios
        let test_cases = vec![
            // Simple multi-task
            (
                vec!["task1", "task2"],
                vec!["task1", "task2"],
                vec![vec!["task1"], vec!["task2"]]
            ),
            // Multi-task with args
            (
                vec!["task1", "-a", "val1", "task2", "-b", "val2"],
                vec!["task1", "task2"],
                vec![vec!["task1", "-a", "val1"], vec!["task2", "-b", "val2"]]
            ),
            // Tasks with complex arguments
            (
                vec!["build", "--release", "--target", "x86_64", "test", "--verbose"],
                vec!["build", "test"],
                vec![vec!["build", "--release", "--target", "x86_64"], vec!["test", "--verbose"]]
            ),
        ];

        for (args, task_names, expected) in test_cases {
            let args: Vec<String> = args.into_iter().map(String::from).collect();
            let result = partitions(&args, &task_names);
            assert_eq!(result, expected, "Failed for args: {:?}", args);
        }
    }

    #[test]
    fn test_edge_case_partitions() {
        // Test edge cases for partitioning

        // Empty args
        let result = partitions(&vec![], &["task1"]);
        assert_eq!(result, Vec::<Vec<String>>::new());

        // No matching tasks
        let args = vec_of_strings!["arg1", "arg2", "arg3"];
        let result = partitions(&args, &["task1", "task2"]);
        assert_eq!(result, Vec::<Vec<String>>::new());

        // Single task at beginning
        let args = vec_of_strings!["task1", "arg1", "arg2"];
        let result = partitions(&args, &["task1"]);
        assert_eq!(result, vec![vec!["task1", "arg1", "arg2"]]);

        // Multiple tasks with no args
        let args = vec_of_strings!["task1", "task2", "task3"];
        let result = partitions(&args, &["task1", "task2", "task3"]);
        assert_eq!(result, vec![
            vec!["task1"],
            vec!["task2"],
            vec!["task3"]
        ]);
    }

    #[test]
    fn test_task_name_validation() {
        // Test various task name formats
        let valid_task_names = vec!["hello", "build-all", "test_integration", "deploy.prod"];
        let args: Vec<String> = valid_task_names.iter().map(|s| s.to_string()).collect();

        for task_name in &valid_task_names {
            let result = partitions(&args, &[task_name]);
            assert!(!result.is_empty(), "Should find task: {}", task_name);
        }
    }

    #[test]
    fn test_argument_parsing_edge_cases() {
        // Test edge cases in argument parsing

        // Arguments that look like task names but aren't
        let args = vec_of_strings!["real-task", "--fake-task", "value"];
        let result = partitions(&args, &["real-task"]);
        assert_eq!(result, vec![vec!["real-task", "--fake-task", "value"]]);

        // Task names that look like flags
        let args = vec_of_strings!["--task", "-t", "value"];
        let result = partitions(&args, &["--task"]);
        assert_eq!(result, vec![vec!["--task", "-t", "value"]]);

        // Mixed case scenarios
        let args = vec_of_strings!["Task1", "task1", "TASK1"];
        let result = partitions(&args, &["task1"]);
        assert_eq!(result, vec![vec!["task1", "TASK1"]]);
    }

    #[test]
    fn test_config_loading_scenarios() -> Result<()> {
        // Test different config loading scenarios

        // Test with None (no config) -- should return OttofileNotFound error
        let err = Parser::load_config_from_path(None).unwrap_err();
        let is_ottofile_not_found = err
            .root_cause()
            .downcast_ref::<super::OttofileNotFound>()
            .is_some();
        assert!(is_ottofile_not_found, "Should return OttofileNotFound error");

        Ok(())
    }

    #[test]
    fn test_task_spec_creation() {
        let task_spec = TaskSpec {
            name: "test_task".to_string(),
            action: "echo test".to_string(),
            before: vec!["dep1".to_string(), "before1".to_string()],
            after: vec!["after1".to_string()],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: HashMap::new(),
            help: Some("Test task".to_string()),
            timeout: Some(30),
        };

        let spec = Task::from_task(&task_spec);

        // Should include before tasks as task_deps
        assert_eq!(spec.task_deps, vec!["dep1", "before1"]);
        assert_eq!(spec.name, "test_task");
        assert_eq!(spec.action, "echo test");
    }

    #[test]
    fn test_comprehensive_help_scenarios() {
        // Test comprehensive help detection scenarios
        let help_scenarios = vec![
            // Top-level help
            (vec!["otto", "--help"], true, false),
            (vec!["otto", "-h"], true, false),

            // Task-level help
            (vec!["otto", "task", "--help"], true, true),
            (vec!["otto", "task", "-h"], true, true),

            // Help mixed with other args
            (vec!["otto", "-j", "4", "--help"], true, false),
            (vec!["otto", "task", "-p", "val", "--help"], true, true),

            // No help
            (vec!["otto", "task"], false, false),
            (vec!["otto", "-j", "4", "task"], false, false),
        ];

        for (args, should_have_help, should_be_task_help) in help_scenarios {
            let args: Vec<String> = args.into_iter().map(String::from).collect();
            let has_help = args.iter().any(|arg| arg == "--help" || arg == "-h");
            assert_eq!(has_help, should_have_help, "Help detection failed for: {:?}", args);

            if should_have_help && should_be_task_help {
                // Should have a task name before help
                let help_pos = args.iter().position(|arg| arg == "--help" || arg == "-h").unwrap();
                assert!(help_pos > 1, "Task help should have task name before help flag");
            }
        }
    }

    #[test]
    fn test_after_dependencies() {
        // Create a simple test for the dependency computation algorithm
        let mut tasks = HashMap::new();

        // Task A has after: [B] - meaning A runs before B, so B depends on A
        let task_a_spec = TaskSpec {
            name: "A".to_string(),
            action: "echo A".to_string(),
            before: vec![],
            after: vec!["B".to_string()],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        };

        // Task B has before: [C] - meaning C runs before B, so B depends on C
        let task_b_spec = TaskSpec {
            name: "B".to_string(),
            action: "echo B".to_string(),
            before: vec!["C".to_string()],
            after: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        };

        // Task C is simple
        let task_c_spec = TaskSpec {
            name: "C".to_string(),
            action: "echo C".to_string(),
            before: vec![],
            after: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        };

        tasks.insert("A".to_string(), task_a_spec);
        tasks.insert("B".to_string(), task_b_spec);
        tasks.insert("C".to_string(), task_c_spec);

        let pargs = vec![];  // Empty pargs for this test

        let parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config_spec: ConfigSpec {
                otto: OttoSpec::default(),
                tasks: tasks.clone(),
            },
            hash: "test".to_string(),
            args: vec!["otto".to_string()],
            pargs,
            ottofile: None,
        };

        // Test the dependency computation directly
        let task_deps = parser.compute_task_deps().unwrap();

        // Verify computed dependencies:
        // A should have no dependencies
        assert_eq!(task_deps.get("A").unwrap().len(), 0, "A should have no dependencies");

        // B should depend on both A (via A.after) and C (via B.before)
        let b_deps = task_deps.get("B").unwrap();
        assert_eq!(b_deps.len(), 2, "B should have 2 dependencies");
        assert!(b_deps.contains("A"), "B should depend on A (from A.after)");
        assert!(b_deps.contains("C"), "B should depend on C (from B.before)");

        // C should have no dependencies
        assert_eq!(task_deps.get("C").unwrap().len(), 0, "C should have no dependencies");

        // Test DAG construction with B (should pull in A and C)
        let dag = parser.process_tasks_with_filter(&[String::from("B")]).unwrap();

        assert_eq!(dag.node_count(), 3, "DAG should contain A, B, C when requesting B");

        let get_task_idx = |name: &str| -> NodeIndex<u32> {
            (0..dag.raw_nodes().len())
                .map(NodeIndex::new)
                .find(|&i| dag[i].name == name)
                .unwrap_or_else(|| panic!("Task {} not found in DAG", name))
        };

        let a_idx = get_task_idx("A");
        let b_idx = get_task_idx("B");
        let c_idx = get_task_idx("C");

        // Verify edges: A -> B and C -> B
        assert!(dag.find_edge(a_idx, b_idx).is_some(), "A should run before B");
        assert!(dag.find_edge(c_idx, b_idx).is_some(), "C should run before B");

        // Test requesting just A (should get only A since A has no dependencies)
        let dag_a = parser.process_tasks_with_filter(&[String::from("A")]).unwrap();
        assert_eq!(dag_a.node_count(), 1, "DAG should contain only A when requesting A (A has no dependencies)");
    }

    #[test]
    fn test_parameter_parsing_regression() {
        // REGRESSION TEST: Ensure parameter parsing works correctly
        // This test covers the exact scenario that was broken during refactor

        let mut tasks = HashMap::new();

        // Create hello task with -g/--greeting parameter
        let hello_task_spec = TaskSpec {
            name: "hello".to_string(),
            action: "echo \"${greeting:-hello}\"".to_string(),
            before: vec![],
            after: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: {
                let mut params = HashMap::new();
                params.insert("greeting".to_string(), ParamSpec {
                    name: "greeting".to_string(),
                    short: Some('g'),
                    long: Some("greeting".to_string()),
                    param_type: ParamType::OPT,
                    dest: None,
                    metavar: None,
                    default: Some("hello".to_string()),
                    constant: Value::Empty,
                    choices: vec!["hello".to_string(), "howdy".to_string()],
                    nargs: Nargs::One,
                    help: Some("override greeting".to_string()),
                    value: Value::Empty,
                });
                params
            },
            help: Some("hello task help".to_string()),
            timeout: None,
        };

        // Create world task with -n/--name parameter
        let world_task_spec = TaskSpec {
            name: "world".to_string(),
            action: "echo \"${name:-world}\"".to_string(),
            before: vec!["hello".to_string()], // world depends on hello
            after: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: {
                let mut params = HashMap::new();
                params.insert("name".to_string(), ParamSpec {
                    name: "name".to_string(),
                    short: Some('n'),
                    long: Some("name".to_string()),
                    param_type: ParamType::OPT,
                    dest: None,
                    metavar: None,
                    default: Some("world".to_string()),
                    constant: Value::Empty,
                    choices: vec![],
                    nargs: Nargs::One,
                    help: Some("override name".to_string()),
                    value: Value::Empty,
                });
                params
            },
            help: Some("world task help".to_string()),
            timeout: None,
        };

        tasks.insert("hello".to_string(), hello_task_spec);
        tasks.insert("world".to_string(), world_task_spec);

        // Simulate the exact command: otto hello -g howdy world -n mundo
        let args = vec!["otto".to_string(), "hello".to_string(), "-g".to_string(), "howdy".to_string(), "world".to_string(), "-n".to_string(), "mundo".to_string()];
        let task_names = vec!["hello", "world"];
        let pargs = partitions(&args[1..].to_vec(), &task_names); // Skip "otto"

        let parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config_spec: ConfigSpec {
                otto: OttoSpec::default(),
                tasks: tasks.clone(),
            },
            hash: "test".to_string(),
            args,
            pargs,
            ottofile: None,
        };

        // Test that parameters are parsed correctly
        let dag = parser.process_tasks_with_filter(&[String::from("world")]).unwrap();

        // Verify both tasks are in DAG (world depends on hello)
        assert_eq!(dag.node_count(), 2, "DAG should contain hello and world");

        // Find task specs
        let hello_spec = dag.raw_nodes().iter()
            .find(|node| node.weight.name == "hello")
            .expect("hello task should be in DAG");
        let world_spec = dag.raw_nodes().iter()
            .find(|node| node.weight.name == "world")
            .expect("world task should be in DAG");

        // Verify hello task has greeting="howdy"
        assert_eq!(
            hello_spec.weight.envs.get("greeting"),
            Some(&"howdy".to_string()),
            "hello task should have greeting=howdy in envs"
        );
        assert_eq!(
            hello_spec.weight.values.get("greeting"),
            Some(&Value::Item("howdy".to_string())),
            "hello task should have greeting=howdy in values"
        );

        // Verify world task has name="mundo"
        assert_eq!(
            world_spec.weight.envs.get("name"),
            Some(&"mundo".to_string()),
            "world task should have name=mundo in envs"
        );
        assert_eq!(
            world_spec.weight.values.get("name"),
            Some(&Value::Item("mundo".to_string())),
            "world task should have name=mundo in values"
        );
    }

    #[test]
    fn test_single_task_parameter_parsing() {
        // Test parameter parsing for a single task
        use crate::cfg::param::{ParamType, Nargs};

        let mut tasks = HashMap::new();

        let test_task_spec = TaskSpec {
            name: "test".to_string(),
            action: "echo test".to_string(),
            before: vec![],
            after: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: {
                let mut params = HashMap::new();
                params.insert("flag".to_string(), ParamSpec {
                    name: "flag".to_string(),
                    short: Some('f'),
                    long: Some("flag".to_string()),
                    param_type: ParamType::OPT,
                    dest: None,
                    metavar: None,
                    default: Some("default".to_string()),
                    constant: Value::Empty,
                    choices: vec![],
                    nargs: Nargs::One,
                    help: Some("test flag".to_string()),
                    value: Value::Empty,
                });
                params
            },
            help: None,
            timeout: None,
        };

        tasks.insert("test".to_string(), test_task_spec);

        // Test with short flag
        let args = vec!["test".to_string(), "-f".to_string(), "short_value".to_string()];
        let pargs = partitions(&args, &["test"]);

        let parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config_spec: ConfigSpec {
                otto: OttoSpec::default(),
                tasks: tasks.clone(),
            },
            hash: "test".to_string(),
            args: vec!["otto".to_string()],
            pargs,
            ottofile: None,
        };

        let dag = parser.process_tasks_with_filter(&[String::from("test")]).unwrap();
        let test_spec = &dag.raw_nodes()[0].weight;

        assert_eq!(test_spec.envs.get("flag"), Some(&"short_value".to_string()));
        assert_eq!(test_spec.values.get("flag"), Some(&Value::Item("short_value".to_string())));

        // Test with long flag - create new parser since we can't mutate the old one
        let args = vec!["test".to_string(), "--flag".to_string(), "long_value".to_string()];
        let pargs = partitions(&args, &["test"]);
        let parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config_spec: ConfigSpec {
                otto: OttoSpec::default(),
                tasks: tasks.clone(),
            },
            hash: "test".to_string(),
            args: vec!["otto".to_string()],
            pargs,
            ottofile: None,
        };

        let dag = parser.process_tasks_with_filter(&[String::from("test")]).unwrap();
        let test_spec = &dag.raw_nodes()[0].weight;

        assert_eq!(test_spec.envs.get("flag"), Some(&"long_value".to_string()));
        assert_eq!(test_spec.values.get("flag"), Some(&Value::Item("long_value".to_string())));
    }

    #[test]
    fn test_parameter_defaults_and_missing_params() {
        // Test parameter defaults and missing parameter scenarios
        use crate::cfg::param::{ParamType, Nargs};

        let mut tasks = HashMap::new();

        let test_task_spec = TaskSpec {
            name: "test".to_string(),
            action: "echo test".to_string(),
            before: vec![],
            after: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: {
                let mut params = HashMap::new();
                params.insert("required".to_string(), ParamSpec {
                    name: "required".to_string(),
                    short: Some('r'),
                    long: Some("required".to_string()),
                    param_type: ParamType::OPT,
                    dest: None,
                    metavar: None,
                    default: None, // No default
                    constant: Value::Empty,
                    choices: vec![],
                    nargs: Nargs::One,
                    help: Some("required parameter".to_string()),
                    value: Value::Empty,
                });
                params.insert("optional".to_string(), ParamSpec {
                    name: "optional".to_string(),
                    short: Some('o'),
                    long: Some("optional".to_string()),
                    param_type: ParamType::OPT,
                    dest: None,
                    metavar: None,
                    default: Some("default_value".to_string()),
                    constant: Value::Empty,
                    choices: vec![],
                    nargs: Nargs::One,
                    help: Some("optional parameter".to_string()),
                    value: Value::Empty,
                });
                params
            },
            help: None,
            timeout: None,
        };

        tasks.insert("test".to_string(), test_task_spec);

        // Test with only required parameter provided
        let args = vec!["test".to_string(), "-r".to_string(), "required_value".to_string()];
        let pargs = partitions(&args, &["test"]);

        let parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config_spec: ConfigSpec {
                otto: OttoSpec::default(),
                tasks: tasks.clone(),
            },
            hash: "test".to_string(),
            args: vec!["otto".to_string()],
            pargs,
            ottofile: None,
        };

        let dag = parser.process_tasks_with_filter(&[String::from("test")]).unwrap();
        let test_spec = &dag.raw_nodes()[0].weight;

        // Should have required parameter
        assert_eq!(test_spec.envs.get("required"), Some(&"required_value".to_string()));
        assert_eq!(test_spec.values.get("required"), Some(&Value::Item("required_value".to_string())));

        // Should have optional parameter with default value
        assert_eq!(test_spec.envs.get("optional"), Some(&"default_value".to_string()));
        assert_eq!(test_spec.values.get("optional"), Some(&Value::Item("default_value".to_string())));
    }

    #[test]
    fn test_file_dependencies_basic() -> Result<()> {
        // Create temporary directory for test files
        let temp_dir = tempfile::TempDir::new()?;
        let temp_path = temp_dir.path();

        // Create test files
        let src_file = temp_path.join("src.c");
        let config_file = temp_path.join("config.h");

        std::fs::write(&src_file, "int main() { return 0; }")?;
        std::fs::write(&config_file, "#define VERSION 1")?;

        // Change to temp directory for relative path resolution
        let original_dir = std::env::current_dir()?;
        std::env::set_current_dir(temp_path)?;

        let task_spec = TaskSpec {
            name: "build".to_string(),
            action: "gcc -o app src.c".to_string(),
            before: vec![],
            after: vec![],
            input: vec!["src.c".to_string(), "config.h".to_string()],
            output: vec!["app".to_string()],
            envs: HashMap::new(),
            params: HashMap::new(),
            help: None,
            timeout: None,
        };

        let spec = Task::from_task(&task_spec);

        // Restore original directory
        std::env::set_current_dir(original_dir)?;

        assert_eq!(spec.name, "build");
        assert_eq!(spec.file_deps.len(), 2);
        assert_eq!(spec.output_deps.len(), 1);

        // Should include input files as dependencies
        assert!(spec.file_deps.iter().any(|f| f.ends_with("src.c")));
        assert!(spec.file_deps.iter().any(|f| f.ends_with("config.h")));

        // Should include output files
        assert!(spec.output_deps.iter().any(|f| f.ends_with("app")));

        Ok(())
    }

    #[test]
    fn test_file_dependencies_empty() {
        // Test task with no file dependencies
        let task_spec = TaskSpec {
            name: "simple".to_string(),
            action: "echo hello".to_string(),
            before: vec![],
            after: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: HashMap::new(),
            help: None,
            timeout: None,
        };

        let spec = Task::from_task(&task_spec);

        assert_eq!(spec.file_deps.len(), 0);
        assert_eq!(spec.output_deps.len(), 0);
        assert_eq!(spec.task_deps.len(), 0);
    }

    #[test]
    fn test_file_dependencies_mixed_with_task_deps() -> Result<()> {
        // Create temporary directory for test files
        let temp_dir = tempfile::TempDir::new()?;
        let temp_path = temp_dir.path();

        // Create test files
        let app_file = temp_path.join("app");
        let config_file = temp_path.join("config.yml");

        std::fs::write(&app_file, "fake binary content")?;
        std::fs::write(&config_file, "server: production")?;

        // Change to temp directory for relative path resolution
        let original_dir = std::env::current_dir()?;
        std::env::set_current_dir(temp_path)?;

        // Test task with both file and task dependencies
        let task_spec = TaskSpec {
            name: "deploy".to_string(),
            action: "deploy.sh app config.yml".to_string(),
            before: vec!["build".to_string(), "test".to_string()],
            after: vec!["cleanup".to_string()],
            input: vec!["app".to_string(), "config.yml".to_string()],
            output: vec!["deployment.log".to_string()],
            envs: HashMap::new(),
            params: HashMap::new(),
            help: None,
            timeout: None,
        };

        let spec = Task::from_task_with_cwd(&task_spec, temp_path);

        // Restore original directory
        std::env::set_current_dir(original_dir)?;

        // Should have both task and file dependencies
        assert_eq!(spec.task_deps, vec!["build", "test"]);
        assert_eq!(spec.file_deps.len(), 2);
        assert_eq!(spec.output_deps.len(), 1);

        // File dependencies should be resolved
        assert!(spec.file_deps.iter().any(|f| f.ends_with("app")));
        assert!(spec.file_deps.iter().any(|f| f.ends_with("config.yml")));
        assert!(spec.output_deps.iter().any(|f| f.ends_with("deployment.log")));

        Ok(())
    }

    #[test]
    fn test_file_dependencies_yaml_config_integration() -> Result<()> {
        // Create isolated temporary directory for this test
        let temp_dir = tempfile::TempDir::new()?;
        let temp_path = temp_dir.path();

        // Store original directory to restore it later
        let original_dir = std::env::current_dir()?;

        // Perform all test logic in an isolated scope
        let result = std::panic::catch_unwind(|| -> Result<()> {
            // Change to temp directory FIRST
            std::env::set_current_dir(temp_path)?;

            // Create test files AFTER changing directory
            let src_dir = temp_path.join("src");
            let include_dir = temp_path.join("include");
            let build_dir = temp_path.join("build");

            std::fs::create_dir_all(&src_dir)?;
            std::fs::create_dir_all(&include_dir)?;
            std::fs::create_dir_all(&build_dir)?;

            // Create source and header files
            std::fs::write(src_dir.join("main.c"), "#include \"app.h\"\nint main() { return 0; }")?;
            std::fs::write(src_dir.join("utils.c"), "#include \"app.h\"\nvoid utils() {}")?;
            std::fs::write(include_dir.join("app.h"), "#ifndef APP_H\n#define APP_H\n#endif")?;
            std::fs::write(include_dir.join("utils.h"), "#ifndef UTILS_H\n#define UTILS_H\n#endif")?;

            // Test file dependencies work end-to-end with YAML config parsing
            let yaml_content = r#"
otto:
  name: "file-deps-test"
  api: 1

tasks:
  compile:
    input:
      - "src/*.c"
      - "include/*.h"
    output:
      - "build/app"
    action: |
      gcc -Iinclude -o build/app src/*.c
    help: "Compile C application"

  test:
    before: ["compile"]
    input:
      - "build/app"
      - "tests/*.txt"
    output:
      - "test_results.log"
    action: |
      ./build/app < tests/input.txt > test_results.log
    help: "Run tests"
"#;

            let config: ConfigSpec = serde_yaml::from_str(yaml_content)?;

            // Test compile task file dependencies
            let compile_task_spec = config.tasks.get("compile").unwrap();
            let compile_spec = Task::from_task_with_cwd(compile_task_spec, temp_path);

            assert_eq!(compile_spec.name, "compile");
            assert!(compile_spec.file_deps.len() >= 4, "Expected at least 4 files, got {}: {:?}", compile_spec.file_deps.len(), compile_spec.file_deps); // main.c, utils.c, app.h, utils.h
            assert_eq!(compile_spec.output_deps.len(), 1);
            assert!(compile_spec.output_deps.iter().any(|f| f.ends_with("build/app")));
            assert!(compile_spec.file_deps.iter().any(|f| f.contains("main.c")));
            assert!(compile_spec.file_deps.iter().any(|f| f.contains("utils.c")));
            assert!(compile_spec.file_deps.iter().any(|f| f.contains("app.h")));

            // Test test task with both task and file dependencies
            let test_task_spec = config.tasks.get("test").unwrap();
            let test_spec = Task::from_task_with_cwd(test_task_spec, temp_path);

            assert_eq!(test_spec.name, "test");
            assert_eq!(test_spec.task_deps, vec!["compile"]);
            assert!(test_spec.file_deps.len() >= 1); // build/app, plus any tests/*.txt that exist
            assert_eq!(test_spec.output_deps.len(), 1);
            assert!(test_spec.output_deps.iter().any(|f| f.ends_with("test_results.log")));

            Ok(())
        });

        // Always restore directory, even if test panicked
        let _ = std::env::set_current_dir(original_dir);

        // Handle panic or error
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(eyre::eyre!("Test panicked")),
        }
    }

    #[test]
    fn test_file_dependencies_special_characters() -> Result<()> {
        // Create isolated temporary directory for this test
        let temp_dir = tempfile::TempDir::new()?;
        let temp_path = temp_dir.path();

        // Store original directory to restore it later
        let original_dir = std::env::current_dir()?;

        // Perform all test logic in an isolated scope
        let result = std::panic::catch_unwind(|| -> Result<()> {
            // Change to temp directory FIRST
            std::env::set_current_dir(temp_path)?;

            // Create files with special characters and spaces AFTER changing directory
            std::fs::write(temp_path.join("file with spaces.txt"), "content")?;
            std::fs::write(temp_path.join("file-with-dashes.log"), "log content")?;
            std::fs::write(temp_path.join("file_with_underscores.cfg"), "config")?;

            let task_spec = TaskSpec {
                name: "special_chars".to_string(),
                action: "process_files.sh".to_string(),
                before: vec![],
                after: vec![],
                input: vec![
                    "file with spaces.txt".to_string(),
                    "file-with-dashes.log".to_string(),
                    "file_with_underscores.cfg".to_string(),
                ],
                output: vec!["output with spaces.txt".to_string()],
                envs: HashMap::new(),
                params: HashMap::new(),
                help: None,
                timeout: None,
            };

            let spec = Task::from_task_with_cwd(&task_spec, temp_path);

            // Should handle special characters properly
            assert_eq!(spec.file_deps.len(), 3);
            assert!(spec.file_deps.iter().any(|f| f.contains("file with spaces.txt")));
            assert!(spec.file_deps.iter().any(|f| f.contains("file-with-dashes.log")));
            assert!(spec.file_deps.iter().any(|f| f.contains("file_with_underscores.cfg")));
            assert_eq!(spec.output_deps.len(), 1);
            assert!(spec.output_deps.iter().any(|f| f.contains("output with spaces.txt")));

            Ok(())
        });

        // Always restore directory, even if test panicked
        let _ = std::env::set_current_dir(original_dir);

        // Handle panic or error
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(eyre::eyre!("Test panicked")),
        }
    }
}
