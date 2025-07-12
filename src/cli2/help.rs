use colored::Colorize;
use crate::cfg::config::ConfigSpec;
use crate::cfg::task::TaskSpec;

pub struct HelpGenerator {
    config: Option<ConfigSpec>,
}

impl HelpGenerator {
    pub fn new(config: Option<ConfigSpec>) -> Self {
        Self { config }
    }

    pub fn generate_main_help(&self) -> String {
        match &self.config {
            Some(config) => self.generate_tasks_help(config),
            None => self.generate_no_config_help(),
        }
    }

    pub fn generate_task_help(&self, task_name: &str) -> Result<String, String> {
        if let Some(ref config) = self.config {
            if let Some(task_spec) = config.tasks.get(task_name) {
                Ok(self.generate_single_task_help(task_name, task_spec))
            } else {
                Err(format!("Task '{}' not found", task_name))
            }
        } else {
            Err("No configuration loaded".to_string())
        }
    }

    fn generate_tasks_help(&self, config: &ConfigSpec) -> String {
        let mut help = String::new();

        // Header
        help.push_str(&format!("{}\n\n", "A task runner".bold()));

        // Usage
        help.push_str(&format!("{}: {} [OPTIONS] [TASKS...]\n\n", "Usage".bold(), "otto".green()));

        // Global options
        help.push_str(&format!("{}:\n", "Options".bold()));
        help.push_str(&format!("  {:<25} {}\n", "-o, --ottofile <PATH>".green(), "path to the ottofile [default: ./]"));
        help.push_str(&format!("  {:<25} {}\n", "-a, --api <URL>".green(), "api url [default: 1]"));
        help.push_str(&format!("  {:<25} {}\n", "-j, --jobs <JOBS>".green(), "number of jobs to run in parallel [default: 32]"));
        help.push_str(&format!("  {:<25} {}\n", "-H, --home <PATH>".green(), "path to the Otto home directory [default: ~/.otto]"));
        help.push_str(&format!("  {:<25} {}\n", "-t, --tasks <TASKS>".green(), "comma separated list of tasks to run [default: *]"));
        help.push_str(&format!("  {:<25} {}\n", "-v, --verbosity <LEVEL>".green(), "verbosity level [default: 1]"));
        help.push_str(&format!("  {:<25} {}\n", "-T, --timeout <SECONDS>".green(), "global timeout in seconds (overrides task-specific timeouts)"));
        help.push_str(&format!("  {:<25} {}\n", "-h, --help".green(), "Print help"));
        help.push_str(&format!("  {:<25} {}\n", "-V, --version".green(), "Print version"));

        // Available tasks
        if !config.tasks.is_empty() {
            help.push_str(&format!("\n{}:\n", "Available Tasks".bold()));

            // Calculate max task name length for alignment
            let max_name_len = config.tasks.keys().map(|name| name.len()).max().unwrap_or(0);
            let padding = std::cmp::max(max_name_len + 2, 20);

            for (task_name, task_spec) in &config.tasks {
                let description = task_spec.help.as_ref()
                    .map(|d| d.as_str())
                    .unwrap_or("No description");

                help.push_str(&format!("  {:<width$} {}\n",
                    task_name.cyan(),
                    description,
                    width = padding
                ));
            }

            help.push_str(&format!("\nFor more information on a specific task, try: {} <TASK> {}\n",
                "otto".green(),
                "--help".cyan()
            ));
        }

        // Footer with logs location
        help.push_str(&format!("\n{}: {}\n",
            "Logs are written to".dimmed(),
            "/home/saidler/.local/share/otto/logs/otto.log".dimmed()
        ));

        help
    }

    fn generate_single_task_help(&self, task_name: &str, task_spec: &TaskSpec) -> String {
        let mut help = String::new();

        // Header
        help.push_str(&format!("{}: {}\n\n", "Task".bold(), task_name.cyan()));

        // Description
        if let Some(ref description) = task_spec.help {
            help.push_str(&format!("{}\n\n", description));
        }

        // Usage
        help.push_str(&format!("{}: {} {} [OPTIONS]\n\n",
            "Usage".bold(),
            "otto".green(),
            task_name.cyan()
        ));

        // Task-specific parameters
        if !task_spec.params.is_empty() {
            help.push_str(&format!("{}:\n", "Arguments".bold()));

            // Calculate max argument name length for alignment
            let max_arg_len = task_spec.params.keys().map(|name| name.len()).max().unwrap_or(0);
            let padding = std::cmp::max(max_arg_len + 4, 20); // +4 for "--" prefix

            for (param_name, param_spec) in &task_spec.params {
                let arg_display = format!("--{}", param_name);
                let mut description = param_spec.help.clone().unwrap_or_else(|| "No description".to_string());

                // Add default value
                if let Some(ref default) = param_spec.default {
                    description.push_str(&format!(" [default: {}]", default));
                }

                // Add required indicator
                if param_spec.default.is_none() {
                    description.push_str(&format!(" {}", "[required]".red()));
                }

                help.push_str(&format!("  {:<width$} {}\n",
                    arg_display.green(),
                    description,
                    width = padding
                ));
            }
        }

        // Dependencies
        if !task_spec.after.is_empty() {
            help.push_str(&format!("\n{}:\n", "Dependencies".bold()));
            for dep in &task_spec.after {
                help.push_str(&format!("  {}\n", dep.cyan()));
            }
        }

        // Global options reminder
        help.push_str(&format!("\n{}: Global options (like --verbosity, --jobs) can be used with any task\n",
            "Note".bold()
        ));
        help.push_str(&format!("For global options, try: {} {}\n",
            "otto".green(),
            "--help".cyan()
        ));

        help
    }

    fn generate_no_config_help(&self) -> String {
        let mut help = String::new();

        // Header
        help.push_str(&format!("{}\n\n", "A task runner".bold()));

        // Usage
        help.push_str(&format!("{}: {} [OPTIONS] [COMMAND]\n\n", "Usage".bold(), "otto".green()));

        // Global options
        help.push_str(&format!("{}:\n", "Options".bold()));
        help.push_str(&format!("  {:<25} {}\n", "-o, --ottofile <PATH>".green(), "path to the ottofile [default: ./]"));
        help.push_str(&format!("  {:<25} {}\n", "-a, --api <URL>".green(), "api url [default: 1]"));
        help.push_str(&format!("  {:<25} {}\n", "-j, --jobs <JOBS>".green(), "number of jobs to run in parallel [default: 32]"));
        help.push_str(&format!("  {:<25} {}\n", "-H, --home <PATH>".green(), "path to the Otto home directory [default: ~/.otto]"));
        help.push_str(&format!("  {:<25} {}\n", "-t, --tasks <TASKS>".green(), "comma separated list of tasks to run [default: *]"));
        help.push_str(&format!("  {:<25} {}\n", "-v, --verbosity <LEVEL>".green(), "verbosity level [default: 1]"));
        help.push_str(&format!("  {:<25} {}\n", "-T, --timeout <SECONDS>".green(), "global timeout in seconds (overrides task-specific timeouts)"));
        help.push_str(&format!("  {:<25} {}\n", "-V, --version".green(), "Print version"));

        // Footer
        help.push_str(&format!("\n{}: {}\n",
            "Logs are written to".dimmed(),
            "/home/saidler/.local/share/otto/logs/otto.log".dimmed()
        ));

        help.push_str(&format!("\n{}: No ottofile found in this directory or any parent directory!\n",
            "ERROR".red().bold()
        ));
        help.push_str("Otto looks for one of the following files in the current or parent directories:\n\n");
        help.push_str("To get started, create an otto.yml file in your project root.\n");

        let config_files = vec![
            "otto.yml",
            ".otto.yml",
            "otto.yaml",
            ".otto.yaml",
            "Ottofile",
            "OTTOFILE"
        ];

        for file in config_files {
            help.push_str(&format!("  - {}\n", file.green()));
        }

        help
    }

    pub fn generate_version_info(&self) -> String {
        format!("{} {}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        )
    }
}
