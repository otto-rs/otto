use std::collections::HashMap;
use crate::cli::NomParser;
use crate::cli::help::HelpGenerator;
use crate::cfg::config::ConfigSpec;
use crate::cfg::task::TaskSpec;
use crate::cfg::param::{ParamSpec, ParamSpecs};

/// Async demo function to be called from main
pub async fn run_demo() {
    demo_nom_parser();
}

/// Demo function showing how to use the nom-based CLI parser
pub fn demo_nom_parser() {
    println!("=== Otto CLI2 (nom-based) Demo ===\n");

    // Create a sample configuration
    let config = create_sample_config();

    // Create parser with configuration
    let mut parser = NomParser::new(Some(config.clone())).unwrap();

    // Test various command line inputs
    let test_commands = vec![
        "",                                     // Empty - should use default
        "--help",                               // Help flag
        "--version",                            // Version flag
        "hello",                               // Simple task
        "hello --greeting=Hi",                 // Task with argument
        "build --release=true",                // Task with boolean
        "deploy --env=staging",                // Task with choice
        "build --release=false hello --greeting=World", // Multiple tasks
        "--jobs=4 --verbosity=2 hello",        // Global options + task
        "hell",                                // Typo - should suggest "hello"
        "unknown_task",                        // Unknown task
        "deploy",                              // Missing required argument
        "deploy --env=invalid",                // Invalid choice
    ];

    println!("Testing various command line inputs:\n");

    for cmd in test_commands {
        println!("Command: {}", if cmd.is_empty() { "(empty)" } else { cmd });
        match parser.parse(cmd) {
            Ok(parsed) => {
                println!("  ✓ Parsed successfully:");
                println!("    Global options: {:?}", parsed.global_options);
                println!("    Tasks: {:?}", parsed.tasks);
            }
            Err(e) => {
                println!("  ✗ Parse error: {}", e);
            }
        }
        println!();
    }

    // Demo help generation
    println!("=== Help Generation Demo ===\n");

    let help_generator = HelpGenerator::new(Some(config.clone()));

    println!("Main help:");
    println!("{}", help_generator.generate_main_help());

    println!("\nTask-specific help for 'hello':");
    match help_generator.generate_task_help("hello") {
        Ok(help) => println!("{}", help),
        Err(e) => println!("Error: {}", e),
    }

    println!("\nNo config help:");
    let no_config_help = HelpGenerator::new(None);
    println!("{}", no_config_help.generate_main_help());
}

fn create_sample_config() -> ConfigSpec {
    let mut tasks = HashMap::new();

    // Hello task with optional greeting parameter
    let mut hello_params = ParamSpecs::new();
    hello_params.insert("greeting".to_string(), ParamSpec {
        name: "greeting".to_string(),
        short: Some('g'),
        long: Some("greeting".to_string()),
        param_type: crate::cfg::param::ParamType::OPT,
        dest: None,
        metavar: None,
        default: Some("Hello".to_string()),
        constant: crate::cfg::param::Value::Empty,
        choices: vec![],
        nargs: crate::cfg::param::Nargs::One,
        help: Some("The greeting to use".to_string()),
        value: crate::cfg::param::Value::Empty,
    });

    tasks.insert("hello".to_string(), TaskSpec {
        name: "hello".to_string(),
        help: Some("Say hello to someone".to_string()),
        after: vec![],
        before: vec![],
        input: vec![],
        output: vec![],
        envs: HashMap::new(),
        params: hello_params,
        action: "echo \"$greeting World!\"".to_string(),
        timeout: None,
    });

    // Build task with boolean release parameter
    let mut build_params = ParamSpecs::new();
    build_params.insert("release".to_string(), ParamSpec {
        name: "release".to_string(),
        short: Some('r'),
        long: Some("release".to_string()),
        param_type: crate::cfg::param::ParamType::FLG,
        dest: None,
        metavar: None,
        default: Some("false".to_string()),
        constant: crate::cfg::param::Value::Empty,
        choices: vec![],
        nargs: crate::cfg::param::Nargs::Zero,
        help: Some("Build in release mode".to_string()),
        value: crate::cfg::param::Value::Empty,
    });

    tasks.insert("build".to_string(), TaskSpec {
        name: "build".to_string(),
        help: Some("Build the project".to_string()),
        after: vec![],
        before: vec![],
        input: vec![],
        output: vec![],
        envs: HashMap::new(),
        params: build_params,
        action: "cargo build".to_string(),
        timeout: None,
    });

    // Deploy task with environment choice parameter
    let mut deploy_params = ParamSpecs::new();
    deploy_params.insert("env".to_string(), ParamSpec {
        name: "env".to_string(),
        short: Some('e'),
        long: Some("env".to_string()),
        param_type: crate::cfg::param::ParamType::OPT,
        dest: None,
        metavar: None,
        default: None, // Required parameter
        constant: crate::cfg::param::Value::Empty,
        choices: vec!["dev".to_string(), "staging".to_string(), "prod".to_string()],
        nargs: crate::cfg::param::Nargs::One,
        help: Some("Environment to deploy to".to_string()),
        value: crate::cfg::param::Value::Empty,
    });

    tasks.insert("deploy".to_string(), TaskSpec {
        name: "deploy".to_string(),
        help: Some("Deploy the application".to_string()),
        after: vec!["build".to_string()], // Depends on build
        before: vec![],
        input: vec![],
        output: vec![],
        envs: HashMap::new(),
        params: deploy_params,
        action: "echo \"Deploying to $env\"".to_string(),
        timeout: None,
    });

    ConfigSpec {
        otto: crate::cfg::otto::default_otto(),
        tasks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_config_creation() {
        let config = create_sample_config();
        assert!(!config.tasks.is_empty());
        assert!(config.tasks.contains_key("hello"));
        assert!(config.tasks.contains_key("build"));
        assert!(config.tasks.contains_key("deploy"));
    }

    #[test]
    fn test_demo_runs_without_panic() {
        // This test just ensures the demo function doesn't panic
        demo_nom_parser();
    }
}
