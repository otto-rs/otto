// Help generation for nom parser
use crate::cfg::config::ConfigSpec;
use colored::Colorize;

pub struct HelpGenerator;

impl HelpGenerator {
    pub fn generate_help(config_spec: &ConfigSpec) -> String {
        let mut help = String::new();
        
        help.push_str(&format!("{}\n", "Otto Task Runner".bold()));
        help.push_str(&format!("{}\n\n", "A modern task runner with dependency management".italic()));
        
        help.push_str(&format!("{}\n", "USAGE:".bold()));
        help.push_str("    otto [OPTIONS] [TASKS...]\n\n");
        
        help.push_str(&format!("{}\n", "OPTIONS:".bold()));
        help.push_str("    -h, --help       Show this help message\n");
        help.push_str("    -V, --version    Show version information\n");
        help.push_str("    -v, --verbose    Increase verbosity\n");
        help.push_str("    -j, --jobs <N>   Number of parallel jobs\n");
        help.push_str("    -f, --file <F>   Specify ottofile path\n\n");
        
        if !config_spec.tasks.is_empty() {
            help.push_str(&format!("{}\n", "TASKS:".bold()));
            for (name, task) in &config_spec.tasks {
                let help_text = task.help.as_deref().unwrap_or("No description");
                help.push_str(&format!("    {:<15} {}\n", name.green(), help_text));
            }
        }
        
        help
    }
    
    pub fn generate_task_help(config_spec: &ConfigSpec, task_name: &str) -> String {
        if let Some(task) = config_spec.tasks.get(task_name) {
            let mut help = String::new();
            
            help.push_str(&format!("{} {}\n", "Task:".bold(), task_name.green()));
            if let Some(desc) = &task.help {
                help.push_str(&format!("{}\n\n", desc));
            }
            
            if !task.params.is_empty() {
                help.push_str(&format!("{}\n", "ARGUMENTS:".bold()));
                for (name, param) in &task.params {
                    let help_text = param.help.as_deref().unwrap_or("No description");
                    help.push_str(&format!("    {:<15} {}\n", name.green(), help_text));
                }
            }
            
            help
        } else {
            format!("Task '{}' not found", task_name)
        }
    }
} 