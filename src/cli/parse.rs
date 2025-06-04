//#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{value_parser, Arg, Command};
use daggy::{Dag, NodeIndex};
use expanduser::expanduser;
use eyre::{eyre, Result};
use hex;
use sha2::{Digest, Sha256};

use crate::cfg::config::{Config, Otto, Param, Task, Tasks, Value};
  // Test-only imports

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

#[allow(dead_code)]
fn print_type_of<T>(t: &T)
where
    T: ?Sized + Debug,
{
    println!("type={} value={:#?}", std::any::type_name::<T>(), t);
}

#[allow(dead_code)]
fn format_items(items: &[&str], before: Option<&str>, between: Option<&str>, after: Option<&str>) -> String
where
{
    //if between is not None, then join with between
    //if between is None, then join with ""
    let mut s = between.map_or_else(|| items.join(""), |between| items.join(between));
    //if before is not None, then prepend with before
    if let Some(before) = before {
        s = format!("{before}{s}");
    }
    //if after is not None, then append with after
    if let Some(after) = after {
        s = format!("{s}{after}");
    }
    s
}

// This routine is adapted from the *old* Path's `path_relative_from`
// function, which works differently from the new `relative_from` function.
// In particular, this handles the case on unix where both paths are
// absolute but with only the root as the common directory.
// url: https://stackoverflow.com/a/39343127
#[allow(clippy::similar_names)]
fn path_relative_from(path: &Path, base: &Path) -> Option<PathBuf> {
    use std::path::Component;

    if path.is_absolute() == base.is_absolute() {
        let mut ita = path.components();
        let mut itb = base.components();
        let mut comps: Vec<Component> = vec![];
        loop {
            match (ita.next(), itb.next()) {
                (None, None) => break,
                (Some(a), None) => {
                    comps.push(a);
                    comps.extend(ita.by_ref());
                    break;
                }
                (None, _) => comps.push(Component::ParentDir),
                (Some(a), Some(b)) if comps.is_empty() && a == b => (),
                (Some(a), Some(b)) if b == Component::CurDir => comps.push(a),
                (Some(_), Some(b)) if b == Component::ParentDir => return None,
                (Some(a), Some(_)) => {
                    comps.push(Component::ParentDir);
                    for _ in itb {
                        comps.push(Component::ParentDir);
                    }
                    comps.push(a);
                    comps.extend(ita.by_ref());
                    break;
                }
            }
        }
        let val: PathBuf = comps.iter().map(|c| c.as_os_str()).collect();
        if val == Path::new("") {
            Some(PathBuf::from(path))
        } else {
            Some(comps.iter().map(|c| c.as_os_str()).collect())
        }
    } else if path.is_absolute() {
        Some(PathBuf::from(path))
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskSpec {
    pub name: String,
    pub deps: Vec<String>,
    pub envs: HashMap<String, String>,
    pub values: HashMap<String, Value>,
    pub action: String,
    pub hash: String,
}

impl TaskSpec {
    #[must_use]
    pub fn new(
        name: String,
        deps: Vec<String>,
        envs: HashMap<String, String>,
        values: HashMap<String, Value>,
        action: String,
    ) -> Self {
        let hash = calculate_hash(&action);
        Self {
            name,
            deps,
            envs,
            values,
            action,
            hash,
        }
    }
    #[must_use]
    pub fn from_task(task: &Task) -> Self {
        let name = task.name.clone();
        let mut deps = task.deps.clone();
        // Add before dependencies - tasks that must complete before this one
        deps.extend(task.before.iter().cloned());
        // Note: We do NOT add after tasks here since they depend on us, not vice versa
        let envs = HashMap::new();
        let values = HashMap::new();
        let action = task.action.trim().to_string();  // Trim whitespace from script content
        Self::new(name, deps, envs, values, action)
    }
}

pub struct Parser {
    prog: String,
    cwd: PathBuf,
    user: String,
    config: Config,
    hash: String,
    args: Vec<String>,
    pargs: Vec<Vec<String>>,
    ottofile: Option<PathBuf>,
}

fn indices(args: &[String], task_names: &[&str]) -> Vec<usize> {
    let mut indices = vec![0];
    for (i, arg) in args.iter().enumerate() {
        if task_names.contains(&arg.as_str()) {
            indices.push(i);
        }
    }
    indices
}

fn partitions(args: &Vec<String>, task_names: &[&str]) -> Vec<Vec<String>> {
    let mut partitions = vec![];
    let mut end = args.len();
    for index in indices(args, task_names).iter().rev() {
        partitions.insert(0, args[*index..end].to_vec());
        end = *index;
    }
    partitions
}

impl Parser {
    pub fn new(args: Vec<String>) -> Result<Self> {
        let mut args = args;
        let prog = std::env::current_exe()?
            .file_name()
            .and_then(OsStr::to_str)
            .map_or_else(|| "otto".to_string(), std::string::ToString::to_string);
        let cwd = env::current_dir()?;
        let user = env::var("USER")?;
        let (config, hash, ottofile) = Self::load_config(&mut args)?;
        let task_names: Vec<&str> = config.tasks.keys().map(std::string::String::as_str).collect();
        let pargs = partitions(&args, &task_names);
        Ok(Self {
            prog,
            cwd,
            user,
            config,
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
        let cwd = env::current_dir()?;
        for ottofile in OTTOFILES {
            let ottofile_path = path.join(ottofile);
            if ottofile_path.exists() {
                let p =
                    path_relative_from(&ottofile_path, &cwd).ok_or_else(|| eyre!("could not find relative path"))?;
                return Ok(Some(p));
            }
        }
        let Some(parent) = path.parent() else { return Ok(None)};
        if parent == Path::new("/") {
            return Ok(None);
        }
        Self::find_ottofile(parent)
    }

    fn divine_ottofile(value: String) -> Result<Option<PathBuf>> {
        let mut path = expanduser(value)?;
        path = fs::canonicalize(path)?;
        if path.is_dir() {
            return Self::find_ottofile(&path);
        }
        Ok(Some(path))
    }

    fn load_config(args: &mut Vec<String>) -> Result<(Config, String, Option<PathBuf>)> {
        // Look for either "-o" or "--ottofile" in the argument list.
        let index = args
            .iter()
            .position(|x| x == "-o" || x == "--ottofile");
        let value = index.map_or_else(
            || env::var("OTTOFILE").unwrap_or_else(|_| "./".to_owned()),
            |index| {
                let value = args[index + 1].clone();
                args.remove(index);
                args.remove(index);
                value
            },
        );
        if let Some(ottofile) = Self::divine_ottofile(value)? {
            let content = fs::read_to_string(&ottofile)?;
            let hash = calculate_hash(&content);
            let config: Config = serde_yaml::from_str(&content)?;
            Ok((config, hash, Some(ottofile)))
        } else {
            Ok((Config::default(), DEFAULT_HASH.to_owned(), None))
        }
    }

    fn otto_to_command(otto: &Otto, tasks: &Tasks) -> Command {
        let mut command = Command::new(&otto.name)
            .bin_name(&otto.name)
            .about(&otto.about)
            .arg(
                Arg::new("ottofile")
                    .short('o')
                    .long("ottofile")
                    //.takes_value(true)
                    .value_name("PATH")
                    .default_value("./")
                    .help("path to the ottofile"),
            )
            .arg(
                Arg::new("api")
                    .short('a')
                    .long("api")
                    //.takes_value(true)
                    .value_name("URL")
                    .default_value(&otto.api)
                    .help("api url"),
            )
            .arg(
                Arg::new("jobs")
                    .short('j')
                    .long("jobs")
                    .value_name("JOBS")
                    .default_value(&otto.jobs.to_string())
                    .value_parser(value_parser!(usize))
                    .help("number of jobs to run in parallel"),
            )
            .arg(
                Arg::new("home")
                    .short('H')
                    .long("home")
                    .value_name("PATH")
                    .default_value(&otto.home)
                    .help("path to the Otto home directory"),
            )
            .arg(
                Arg::new("tasks")
                    .short('t')
                    .long("tasks")
                    //.takes_value(true)
                    .value_name("TASKS")
                    .default_values(&otto.tasks)
                    .help("comma separated list of tasks to run"),
            )
            .arg(
                Arg::new("verbosity")
                    .short('v')
                    .long("verbosity")
                    //.takes_value(true)
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
            );
        for task in tasks.values() {
            command = command.subcommand(Self::task_to_command(task));
        }
        command
    }

    fn task_to_command(task: &Task) -> Command {
        let mut command = Command::new(&task.name).bin_name(&task.name);
        if let Some(task_help) = &task.help {
            command = command.about(task_help);
        }
        for param in task.params.values() {
            command = command.arg(Self::param_to_arg(param));
        }
        command
    }

    fn param_to_arg(param: &Param) -> Arg {
        let mut arg = Arg::new(&param.name);
        if let Some(short) = param.short {
            arg = arg.short(short);
        }
        if let Some(long) = &param.long {
            arg = arg.long(long);
        }
        // if param.param_type == ParamType::OPT {
        //     arg = arg.takes_value(true);
        // }
        if let Some(help) = &param.help {
            arg = arg.help(help);
        }
        if let Some(default) = &param.default {
            arg = arg.default_value(default);
        }
        arg
    }

    pub fn parse(&mut self) -> Result<(Otto, DAG<TaskSpec>, String, Option<PathBuf>)> {
        // Process the otto command arguments
        let mut otto = self.process_args()?;

        // Collect the first item in each parg, skipping the first one.
        let configured_tasks = self.pargs.iter().skip(1).map(|p| p[0].clone()).collect::<Vec<String>>();

        // If tasks were passed as arguments, they replace the default tasks.
        // Otherwise, use the default tasks from the config.
        if configured_tasks.is_empty() {
            otto.tasks = self.config.otto.tasks.clone();
        } else {
            otto.tasks = configured_tasks;
        }

        // Process only the requested tasks and their dependencies
        let tasks = self.process_tasks_with_filter(&otto.tasks)?;

        // Return all jobs from the Ottofile, and the updated Otto struct
        Ok((otto, tasks, self.hash.clone(), self.ottofile.clone()))
    }

    fn process_tasks_with_filter(&self, requested_tasks: &[String]) -> Result<DAG<TaskSpec>> {
        // Initialize an empty Dag and an index map
        let mut dag: DAG<TaskSpec> = DAG::new();
        let mut indices: HashMap<String, NodeIndex<u32>> = HashMap::new();

        // Helper function to recursively add a task and its dependencies
        fn add_task_and_deps(
            task_name: &str,
            config: &Config,
            dag: &mut DAG<TaskSpec>,
            indices: &mut HashMap<String, NodeIndex<u32>>,
            pargs: &[Vec<String>],
        ) -> Result<()> {
            // Skip if already added
            if indices.contains_key(task_name) {
                return Ok(());
            }

            let task = config.tasks.get(task_name)
                .ok_or_else(|| eyre!("Task {} not found", task_name))?;

            // First add all dependencies recursively
            for dep in task.deps.iter().chain(task.before.iter()) {
                add_task_and_deps(dep, config, dag, indices, pargs)?;
            }

            // Create the task spec
            let mut spec = TaskSpec::from_task(task);

            // Apply default values and command line parameters
            for (name, param) in &task.params {
                if let Some(default_value) = &param.default {
                    let value = Value::Item(default_value.clone());
                    spec.values.insert(name.clone(), value);
                }
            }

            // Check for command line parameters
            if let Some(task_args) = pargs[1..].iter().find(|partition| partition[0] == task.name) {
                let task_command = Parser::task_to_command(task);
                let matches = task_command.get_matches_from(task_args);

                for param in task.params.values() {
                    if let Some(value) = matches.get_one::<String>(param.name.as_str()) {
                        spec.values.insert(param.name.clone(), Value::Item(value.to_string()));
                        // Also add to environment variables
                        spec.envs.insert(param.name.clone(), value.to_string());
                    }
                }
            }

            // Add the task to the DAG
            let index = dag.add_node(spec.clone());
            indices.insert(task_name.to_string(), index);

            // Add edges for dependencies
            for dep_name in task.deps.iter().chain(task.before.iter()) {
                let dep_index = indices.get(dep_name).expect("Dependency should exist");
                dag.add_edge(*dep_index, index, ())?;
            }

            // Add edges for 'after' dependencies
            for after_name in &task.after {
                add_task_and_deps(after_name, config, dag, indices, pargs)?;
                let after_index = indices.get(after_name).expect("After task should exist");
                dag.add_edge(index, *after_index, ())?;
            }

            Ok(())
        }

        // Add each requested task and its dependencies
        for task_name in requested_tasks {
            add_task_and_deps(task_name, &self.config, &mut dag, &mut indices, &self.pargs)?;
        }

        Ok(dag)
    }

    fn handle_no_input(&self) {
        // Create a default otto command with no tasks
        let otto_command = Self::otto_to_command(&self.config.otto, &HashMap::new());
        otto_command.get_matches_from(["otto", "--help"]);
    }

    fn process_args(&mut self) -> Result<Otto> {
        let mut otto = self.config.otto.clone();

        // if config.tasks is empty, then show default help for 'otto' command and exit
        if self.config.tasks.is_empty() {
            self.handle_no_input();
        }

        // Create command with all task subcommands
        let command = Self::otto_to_command(&otto, &self.config.tasks);
        let matches = command.get_matches_from(&self.args);

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

        // right now args will have at least one element, the name of the otto binary
        // tasks will be ["*"]
        // so this logic is bunk at the moment
        if self.args.len() == 1 && otto.tasks.is_empty() {
            return Err(eyre!("No tasks configified"));
        }

        Ok(otto)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::collections::HashSet;
    use crate::cfg::param::{ParamType, Nargs};  // Test-only imports

    #[test]
    fn test_indices() {
        let args = vec_of_strings!["arg1", "task1", "arg2", "task2", "arg3",];
        let task_names = &["task1", "task2"];
        let expected = vec![0, 1, 3];
        assert_eq!(indices(&args, task_names), expected);
    }

    #[test]
    fn test_partitions() {
        let args = vec_of_strings!["arg1", "task1", "arg2", "task2", "arg3",];
        let task_names = vec!["task1", "task2"];
        assert_eq!(
            partitions(&args, &task_names),
            vec![vec!["arg1"], vec!["task1", "arg2"], vec!["task2", "arg3"]]
        );
    }

    #[test]
    fn test_parser_new() {
        let args = vec![];
        assert!(Parser::new(args).is_ok());
    }

    fn generate_test_otto() -> Otto {
        Otto {
            name: "otto".to_string(),
            home: "~/.otto".to_string(),
            about: "A task runner".to_string(),
            api: "http://localhost:8000".to_string(),
            jobs: num_cpus::get(),
            verbosity: 1,
            tasks: vec!["build".to_string()],
            timeout: None,
        }
    }

    #[test]
    fn test_handle_no_input_no_ottofile() {
        let args = vec![];
        let parser = Parser::new(args).unwrap();

        // Rename or delete Ottofile in current directory if it exists
        if Path::new("Ottofile").exists() {
            fs::rename("Ottofile", "Ottofile.bak").unwrap();
        }

        // Call handle_no_input and check that it doesn't panic
        let result = std::panic::catch_unwind(|| parser.handle_no_input());
        assert!(result.is_ok(), "handle_no_input panicked when no Ottofile was present");

        // Restore Ottofile
        if Path::new("Ottofile.bak").exists() {
            fs::rename("Ottofile.bak", "Ottofile").unwrap();
        }
    }

    #[test]
    fn test_parse_no_args() {
        let otto = generate_test_otto();
        println!("generated otto: {otto:#?}");

        let args = vec!["otto".to_string()];
        let pargs = partitions(&args, &["build"]);

        let mut parser = Parser {
            hash: DEFAULT_HASH.to_string(),
            prog: "otto".to_string(),
            cwd: env::current_dir().unwrap(),
            user: env::var("USER").unwrap(),
            config: Config {
                otto,
                tasks: HashMap::new(),
            },
            args,
            pargs,
            ottofile: None,
        };

        let result = parser.parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_with_args() {
        let otto = generate_test_otto();
        
        // Create a build task that matches what's expected
        let build_task = Task {
            name: "build".to_string(),
            action: "echo build".to_string(),
            deps: vec![],
            before: vec![],
            after: vec![],
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        };

        let mut tasks = HashMap::new();
        tasks.insert(build_task.name.clone(), build_task);

        let args = vec!["otto".to_string(), "build".to_string()];
        let pargs = partitions(&args, &["build"]);

        let mut parser = Parser {
            prog: "otto".to_string(),
            hash: DEFAULT_HASH.to_string(),
            cwd: env::current_dir().unwrap(),
            user: env::var("USER").unwrap(),
            config: Config {
                otto: otto.clone(),
                tasks: tasks.clone(),
            },
            args,
            pargs,
            ottofile: None,
        };

        let result = parser.parse().unwrap();
        let (otto, dag, _, _) = result;

        assert_eq!(otto, otto, "comparing otto struct");

        // We expect the same number of jobs as tasks
        assert_eq!(dag.node_count(), tasks.len(), "comparing tasks length");

        // Use node_weight to get Job data
        let first_node_index = NodeIndex::new(0);
        let first_task = dag.node_weight(first_node_index).unwrap();

        // Assert job name
        assert_eq!(first_task.name, "build".to_string(), "comparing task name");
    }

    #[test]
    fn test_task_dependencies() {
        let task = Task {
            name: "main".to_string(),
            action: "echo main".to_string(),
            deps: vec!["dep1".to_string()],
            before: vec!["before1".to_string()],
            after: vec!["after1".to_string()],
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        };

        // Test that TaskSpec::from_task only includes deps and before tasks
        let spec = TaskSpec::from_task(&task);
        let expected_deps: HashSet<String> = vec!["dep1".to_string(), "before1".to_string()]
            .into_iter()
            .collect();
        let actual_deps: HashSet<String> = spec.deps.into_iter().collect();
        assert_eq!(actual_deps, expected_deps, "TaskSpec should only include deps and before tasks");

        // Test DAG construction with all dependency types
        let mut tasks = HashMap::new();
        tasks.insert(task.name.clone(), task.clone());

        // Add the dependency tasks
        for name in ["dep1", "before1", "after1"] {
            let dep_task = Task {
                name: name.to_string(),
                action: format!("echo {}", name),
                deps: vec![],
                before: vec![],
                after: vec![],
                params: HashMap::new(),
                help: None,
                timeout: Some(10),
            };
            tasks.insert(name.to_string(), dep_task);
        }

        let args = vec!["otto".to_string()];
        let pargs = vec![args.clone()];  // Initialize pargs with just the program name

        let parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config: Config {
                otto: Otto::default(),
                tasks,
            },
            hash: "test".to_string(),
            args,
            pargs,
            ottofile: None,
        };

        let dag = parser.process_tasks_with_filter(&[String::from("main")]).unwrap();

        // Verify edges in the DAG
        let main_idx = (0..dag.raw_nodes().len())
            .map(NodeIndex::new)
            .find(|&i| dag[i].name == "main")
            .expect("Main task not found in DAG");
        let dep1_idx = (0..dag.raw_nodes().len())
            .map(NodeIndex::new)
            .find(|&i| dag[i].name == "dep1")
            .expect("dep1 task not found in DAG");
        let before1_idx = (0..dag.raw_nodes().len())
            .map(NodeIndex::new)
            .find(|&i| dag[i].name == "before1")
            .expect("before1 task not found in DAG");
        let after1_idx = (0..dag.raw_nodes().len())
            .map(NodeIndex::new)
            .find(|&i| dag[i].name == "after1")
            .expect("after1 task not found in DAG");

        // Check that dep1 and before1 are dependencies of main
        assert!(dag.find_edge(dep1_idx, main_idx).is_some(), "dep1 should be a dependency of main");
        assert!(dag.find_edge(before1_idx, main_idx).is_some(), "before1 should be a dependency of main");

        // Check that main is a dependency of after1
        assert!(dag.find_edge(main_idx, after1_idx).is_some(), "main should be a dependency of after1");
    }

    #[test]
    fn test_task_selection() -> Result<()> {
        // Create a config with multiple tasks
        let mut tasks = HashMap::new();
        
        // Task with no dependencies
        tasks.insert("standalone".to_string(), Task {
            name: "standalone".to_string(),
            action: "echo standalone".to_string(),
            deps: vec![],
            before: vec![],
            after: vec![],
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        });

        // Task with dependencies
        tasks.insert("dependent".to_string(), Task {
            name: "dependent".to_string(),
            action: "echo dependent".to_string(),
            deps: vec!["dependency1".to_string()],
            before: vec!["dependency2".to_string()],
            after: vec!["after1".to_string()],
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        });

        // Dependencies
        tasks.insert("dependency1".to_string(), Task {
            name: "dependency1".to_string(),
            action: "echo dep1".to_string(),
            deps: vec![],
            before: vec![],
            after: vec![],
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        });

        tasks.insert("dependency2".to_string(), Task {
            name: "dependency2".to_string(),
            action: "echo dep2".to_string(),
            deps: vec![],
            before: vec![],
            after: vec![],
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        });

        tasks.insert("after1".to_string(), Task {
            name: "after1".to_string(),
            action: "echo after1".to_string(),
            deps: vec![],
            before: vec![],
            after: vec![],
            params: HashMap::new(),
            help: None,
            timeout: Some(10),
        });

        let otto = Otto {
            name: "otto".to_string(),
            about: "test".to_string(),
            api: "1".to_string(),
            jobs: 1,
            home: "~/.otto".to_string(),
            tasks: vec!["standalone".to_string()],
            verbosity: 1,
            timeout: None,
        };

        let config = Config { otto, tasks };

        // Test 1: Running standalone task
        let args = vec!["otto".to_string(), "standalone".to_string()];
        let mut parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config: config.clone(),
            hash: "test".to_string(),
            args: args.clone(),
            pargs: partitions(&args, &["standalone"]),
            ottofile: None,
        };

        let (_, dag, _, _) = parser.parse()?;
        assert_eq!(dag.node_count(), 1, "Standalone task should create exactly one node");
        assert_eq!(dag.edge_count(), 0, "Standalone task should have no edges");

        // Test 2: Running dependent task
        let args = vec!["otto".to_string(), "dependent".to_string()];
        let mut parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config: config.clone(),
            hash: "test".to_string(),
            args: args.clone(),
            pargs: partitions(&args, &["dependent"]),
            ottofile: None,
        };

        let (_, dag, _, _) = parser.parse()?;
        assert_eq!(dag.node_count(), 4, "Should include dependent task and its dependencies");
        
        // Verify the correct edges exist
        let mut found_edges = HashSet::new();
        for edge in dag.raw_edges() {
            let from = &dag.raw_nodes()[edge.source().index()].weight.name;
            let to = &dag.raw_nodes()[edge.target().index()].weight.name;
            found_edges.insert((from.clone(), to.clone()));
        }

        assert!(found_edges.contains(&("dependency1".to_string(), "dependent".to_string())), 
            "Should have edge from dependency1 to dependent");
        assert!(found_edges.contains(&("dependency2".to_string(), "dependent".to_string())), 
            "Should have edge from dependency2 to dependent");
        assert!(found_edges.contains(&("dependent".to_string(), "after1".to_string())), 
            "Should have edge from dependent to after1");

        Ok(())
    }

    #[test]
    fn test_parameter_passing() -> Result<()> {
        // Create a task with parameters
        let mut params = HashMap::new();
        params.insert("-g|--greeting".to_string(), Param {
            name: "greeting".to_string(),
            short: Some('g'),
            long: Some("greeting".to_string()),
            default: Some("hello".to_string()),
            choices: vec!["hello".to_string(), "howdy".to_string()],
            help: Some("greeting help".to_string()),
            param_type: ParamType::OPT,
            dest: None,
            metavar: None,
            constant: Value::Empty,
            nargs: Nargs::One,
            value: Value::Empty,
        });

        let mut tasks = HashMap::new();
        tasks.insert("greet".to_string(), Task {
            name: "greet".to_string(),
            action: "echo ${greeting}".to_string(),
            deps: vec![],
            before: vec![],
            after: vec![],
            params,
            help: None,
            timeout: Some(10),
        });

        let otto = Otto {
            name: "otto".to_string(),
            about: "test".to_string(),
            api: "1".to_string(),
            jobs: 1,
            home: "~/.otto".to_string(),
            tasks: vec!["greet".to_string()],
            verbosity: 1,
            timeout: None,
        };

        let config = Config { otto, tasks };

        // Test 1: Default parameter value
        let args = vec!["otto".to_string(), "greet".to_string()];
        let mut parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config: config.clone(),
            hash: "test".to_string(),
            args: args.clone(),
            pargs: partitions(&args, &["greet"]),
            ottofile: None,
        };

        let (_, dag, _, _) = parser.parse()?;
        let task = &dag.raw_nodes()[0].weight;
        assert_eq!(task.values.get("greeting").unwrap(), &Value::Item("hello".to_string()),
            "Default parameter value not set correctly");

        // Test 2: Override parameter value
        let args = vec!["otto".to_string(), "greet".to_string(), "-g".to_string(), "howdy".to_string()];
        let mut parser = Parser {
            prog: "otto".to_string(),
            cwd: PathBuf::from("/"),
            user: "test".to_string(),
            config: config.clone(),
            hash: "test".to_string(),
            args: args.clone(),
            pargs: partitions(&args, &["greet"]),
            ottofile: None,
        };

        let (_, dag, _, _) = parser.parse()?;
        let task = &dag.raw_nodes()[0].weight;
        assert_eq!(task.values.get("greeting").unwrap(), &Value::Item("howdy".to_string()),
            "Parameter override not working");
        assert_eq!(task.envs.get("greeting").unwrap(), "howdy",
            "Parameter not added to environment variables");

        Ok(())
    }
}
