use std::collections::HashMap;
use std::path::PathBuf;
use regex::Regex;
use lru::LruCache;
use std::num::NonZeroUsize;

use crate::cli2::types::{
    ParsedCommand, ParsedTask, GlobalOptions, Token
};
use crate::cli2::error::{ParseError};
use crate::cli2::validation::{KeywordValidator, ArgumentValidator, suggest_similar_task_names};
use crate::cfg::config::ConfigSpec;

pub struct NomParser {
    config: Option<ConfigSpec>,
    keyword_validator: KeywordValidator,
    // Performance optimizations
    task_name_regex: Option<Regex>,
    argument_regex: Regex,
    #[allow(dead_code)]
    parser_cache: LruCache<String, ()>, // Placeholder for cached parsers
}

impl NomParser {
    pub fn new(config: Option<ConfigSpec>) -> Result<Self, ParseError> {
        let keyword_validator = KeywordValidator::new();

        // Validate config for keyword collisions
        if let Some(ref cfg) = config {
            keyword_validator.validate_config(cfg)
                .map_err(|errors| ParseError::CollisionError { errors })?;
        }

        // Pre-compile regex patterns for performance
        let task_name_regex = if let Some(ref cfg) = config {
            let task_names: Vec<&str> = cfg.tasks.keys().map(|s| s.as_str()).collect();
            if !task_names.is_empty() {
                let pattern = format!("^({})", task_names.join("|"));
                Some(Regex::new(&pattern).map_err(|e| ParseError::ParsingError {
                    input: pattern,
                    position: 0,
                    expected: format!("Valid regex pattern: {}", e),
                })?)
            } else {
                None
            }
        } else {
            None
        };

        let argument_regex = Regex::new(r"^--([a-zA-Z][a-zA-Z0-9_-]*)=(.*)$")
            .map_err(|e| ParseError::ParsingError {
                input: "argument regex".to_string(),
                position: 0,
                expected: format!("Valid regex pattern: {}", e),
            })?;

        Ok(Self {
            config,
            keyword_validator,
            task_name_regex,
            argument_regex,
            parser_cache: LruCache::new(NonZeroUsize::new(50).unwrap()),
        })
    }

    pub fn parse(&mut self, input: &str) -> Result<ParsedCommand, ParseError> {
        let input = input.trim();

        // Handle special cases first
        if input.is_empty() {
            return Ok(ParsedCommand {
                global_options: GlobalOptions::default(),
                tasks: self.get_default_tasks(),
            });
        }

        // Check for help/version flags
        if input == "--help" || input == "-h" {
            return Ok(ParsedCommand {
                global_options: GlobalOptions { help: true, ..Default::default() },
                tasks: vec![],
            });
        }

        if input == "--version" || input == "-V" {
            return Ok(ParsedCommand {
                global_options: GlobalOptions { version: true, ..Default::default() },
                tasks: vec![],
            });
        }

        // Try fast path parsing first
        self.quick_parse(input)
    }

    fn quick_parse(&mut self, input: &str) -> Result<ParsedCommand, ParseError> {
        // Tokenize using regex for common patterns
        let tokens = self.tokenize_fast(input)?;
        self.parse_tokens(tokens)
    }

    fn tokenize_fast(&self, input: &str) -> Result<Vec<Token>, ParseError> {
        let mut tokens = Vec::new();
        let mut remaining = input.trim();

        while !remaining.is_empty() {
            remaining = remaining.trim_start();

            // Try to match arguments with equals sign first (--key=value)
            // But only apply regex to the current argument, not the entire remaining string
            if remaining.starts_with("--") {
                // Find the end of the current argument (next space or end of string)
                let arg_end = remaining.find(' ').unwrap_or(remaining.len());
                let current_arg = &remaining[..arg_end];

                if let Some(captures) = self.argument_regex.captures(current_arg) {
                    let arg_name = captures.get(1).unwrap().as_str();
                    let value = captures.get(2).map(|m| m.as_str().to_string());

                    if self.keyword_validator.global_options.contains(arg_name) {
                        tokens.push(Token::GlobalOption {
                            name: arg_name.to_string(),
                            value,
                        });
                    } else {
                        tokens.push(Token::TaskArgument {
                            name: arg_name.to_string(),
                            value,
                        });
                    }

                    remaining = &remaining[arg_end..];
                    continue;
                }
            }

            // Try to match arguments without equals sign (--key value)
            if remaining.starts_with("--") {
                // Find the end of the argument name
                let arg_end = remaining.find(' ').unwrap_or(remaining.len());
                let arg_full = &remaining[..arg_end];

                // Extract argument name (remove --)
                let arg_name = &arg_full[2..];

                // Check if this is a global option
                if self.keyword_validator.global_options.contains(arg_name) {
                    // Look for the next space-separated value
                    remaining = &remaining[arg_end..].trim_start();

                    let value = if !remaining.is_empty() && !remaining.starts_with("-") {
                        // Find the next space or end of string, but stop at next argument
                        let value_end = remaining.find(' ').unwrap_or(remaining.len());
                        let value_str = &remaining[..value_end];
                        remaining = &remaining[value_end..];
                        Some(value_str.to_string())
                    } else {
                        None
                    };

                    tokens.push(Token::GlobalOption {
                        name: arg_name.to_string(),
                        value,
                    });
                } else {
                    // This is a task argument
                    remaining = &remaining[arg_end..].trim_start();

                    let value = if !remaining.is_empty() && !remaining.starts_with("-") {
                        let value_end = remaining.find(' ').unwrap_or(remaining.len());
                        let value_str = &remaining[..value_end];

                        // Check if this potential value is actually a known task name
                        if let Some(ref config) = self.config {
                            if config.tasks.contains_key(value_str) {
                                // This is a task name, don't consume it as a value
                                None
                            } else {
                                remaining = &remaining[value_end..];
                                Some(value_str.to_string())
                            }
                        } else {
                            remaining = &remaining[value_end..];
                            Some(value_str.to_string())
                        }
                    } else {
                        None
                    };

                    tokens.push(Token::TaskArgument {
                        name: arg_name.to_string(),
                        value,
                    });
                }
                continue;
            }

            // Try to match short flags (-o, -h, etc.)
            if remaining.starts_with('-') && remaining.len() > 1 && !remaining.starts_with("--") {
                let flag_char = remaining.chars().nth(1).unwrap();
                let flag_str = format!("-{}", flag_char);

                // Handle special short flags
                if flag_str == "-h" {
                    tokens.push(Token::Help);
                    remaining = &remaining[2..];
                    continue;
                } else if flag_str == "-V" {
                    tokens.push(Token::Version);
                    remaining = &remaining[2..];
                    continue;
                } else if flag_str == "-o" {
                    // -o is short for --ottofile
                    remaining = &remaining[2..].trim_start();

                    let value = if !remaining.is_empty() {
                        let value_end = remaining.find(' ').unwrap_or(remaining.len());
                        let value_str = &remaining[..value_end];
                        remaining = &remaining[value_end..];
                        Some(value_str.to_string())
                    } else {
                        None
                    };

                    tokens.push(Token::GlobalOption {
                        name: "ottofile".to_string(),
                        value,
                    });
                    continue;
                }
            }

            // Try to match task names
            if let Some(ref regex) = self.task_name_regex {
                if let Some(captures) = regex.captures(remaining) {
                    let task_name = captures.get(1).unwrap().as_str();
                    tokens.push(Token::TaskName(task_name.to_string()));
                    remaining = &remaining[captures.get(0).unwrap().end()..];
                    continue;
                }
            }

            // Handle special tokens
            if remaining.starts_with("--help") {
                tokens.push(Token::Help);
                remaining = &remaining[6..];
                continue;
            }

            if remaining.starts_with("--version") {
                tokens.push(Token::Version);
                remaining = &remaining[9..];
                continue;
            }

            // If we can't match anything, try to extract the next word
            if let Some(space_pos) = remaining.find(' ') {
                let word = &remaining[..space_pos];
                tokens.push(Token::Unknown(word.to_string()));
                remaining = &remaining[space_pos..];
            } else {
                // Last word
                tokens.push(Token::Unknown(remaining.to_string()));
                break;
            }
        }

        Ok(tokens)
    }

    fn parse_tokens(&mut self, tokens: Vec<Token>) -> Result<ParsedCommand, ParseError> {
        let mut global_options = GlobalOptions::default();
        let mut tasks = Vec::new();
        let mut current_task: Option<String> = None;
        let mut current_task_args: HashMap<String, String> = HashMap::new();

        let mut i = 0;
        while i < tokens.len() {
            match &tokens[i] {
                Token::GlobalOption { name, value } => {
                    self.parse_global_option(&mut global_options, name, value.as_deref())?;
                }

                Token::TaskName(name) => {
                    // Finish previous task if any
                    if let Some(task_name) = current_task.take() {
                        let parsed_task = self.finalize_task(task_name, current_task_args)?;
                        tasks.push(parsed_task);
                        current_task_args = HashMap::new();
                    }

                    current_task = Some(name.clone());
                }

                Token::TaskArgument { name, value } => {
                    if current_task.is_some() {
                        let val = if let Some(v) = value {
                            v.clone()
                        } else {
                            // For arguments without values, check if next token is a value
                            // If not, treat as a flag and set to "true"
                            if i + 1 < tokens.len() {
                                match &tokens[i + 1] {
                                    Token::Unknown(next_val) => {
                                        // Check if this looks like a value or another task/argument
                                        if next_val.starts_with('-') ||
                                           (self.config.as_ref().map_or(false, |c| c.tasks.contains_key(next_val))) {
                                            // This is probably another argument or task name, treat current as flag
                                            "true".to_string()
                                        } else {
                                            // This looks like a value, consume it
                                            i += 1; // Skip the next token
                                            next_val.clone()
                                        }
                                    }
                                    _ => {
                                        // Next token is not a value, treat as flag
                                        "true".to_string()
                                    }
                                }
                            } else {
                                // No next token, treat as flag
                                "true".to_string()
                            }
                        };

                        current_task_args.insert(name.clone(), val);
                    } else {
                        return Err(ParseError::InvalidArgument {
                            task_name: "".to_string(),
                            arg_name: name.clone(),
                            error: "Task argument provided without task name".to_string(),
                        });
                    }
                }

                Token::Help => {
                    global_options.help = true;
                }

                Token::Version => {
                    global_options.version = true;
                }

                Token::Unknown(word) => {
                    // Check if this might be a typo of a task name
                    if let Some(ref config) = self.config {
                        let task_names: Vec<String> = config.tasks.keys().cloned().collect();
                        let suggestions = suggest_similar_task_names(word, &task_names);

                        if !suggestions.is_empty() {
                            return Err(ParseError::UnknownTask {
                                name: word.clone(),
                                suggestions,
                            });
                        }
                    }

                    return Err(ParseError::UnknownTask {
                        name: word.clone(),
                        suggestions: vec![],
                    });
                }
            }

            i += 1;
        }

        // Finish last task if any
        if let Some(task_name) = current_task {
            let parsed_task = self.finalize_task(task_name, current_task_args)?;
            tasks.push(parsed_task);
        }

        // If no tasks specified, use defaults
        if tasks.is_empty() && !global_options.help && !global_options.version {
            tasks = self.get_default_tasks();
        }

        Ok(ParsedCommand {
            global_options,
            tasks,
        })
    }

    fn parse_global_option(
        &self,
        global_options: &mut GlobalOptions,
        name: &str,
        value: Option<&str>,
    ) -> Result<(), ParseError> {
        match name {
            "ottofile" => {
                global_options.ottofile = Some(PathBuf::from(value.ok_or_else(|| {
                    ParseError::GlobalOptionError {
                        option_name: name.to_string(),
                        error: "Missing value for --ottofile".to_string(),
                    }
                })?));
            }
            "api" => {
                global_options.api = Some(value.ok_or_else(|| {
                    ParseError::GlobalOptionError {
                        option_name: name.to_string(),
                        error: "Missing value for --api".to_string(),
                    }
                })?.to_string());
            }
            "jobs" => {
                let jobs_str = value.ok_or_else(|| {
                    ParseError::GlobalOptionError {
                        option_name: name.to_string(),
                        error: "Missing value for --jobs".to_string(),
                    }
                })?;
                global_options.jobs = Some(jobs_str.parse().map_err(|_| {
                    ParseError::GlobalOptionError {
                        option_name: name.to_string(),
                        error: format!("Invalid number for --jobs: {}", jobs_str),
                    }
                })?);
            }
            "home" => {
                global_options.home = Some(PathBuf::from(value.ok_or_else(|| {
                    ParseError::GlobalOptionError {
                        option_name: name.to_string(),
                        error: "Missing value for --home".to_string(),
                    }
                })?));
            }
            "tasks" => {
                global_options.tasks = Some(value.ok_or_else(|| {
                    ParseError::GlobalOptionError {
                        option_name: name.to_string(),
                        error: "Missing value for --tasks".to_string(),
                    }
                })?.to_string());
            }
            "verbosity" => {
                let verbosity_str = value.ok_or_else(|| {
                    ParseError::GlobalOptionError {
                        option_name: name.to_string(),
                        error: "Missing value for --verbosity".to_string(),
                    }
                })?;
                global_options.verbosity = Some(verbosity_str.parse().map_err(|_| {
                    ParseError::GlobalOptionError {
                        option_name: name.to_string(),
                        error: format!("Invalid number for --verbosity: {}", verbosity_str),
                    }
                })?);
            }
            "timeout" => {
                let timeout_str = value.ok_or_else(|| {
                    ParseError::GlobalOptionError {
                        option_name: name.to_string(),
                        error: "Missing value for --timeout".to_string(),
                    }
                })?;
                global_options.timeout = Some(timeout_str.parse().map_err(|_| {
                    ParseError::GlobalOptionError {
                        option_name: name.to_string(),
                        error: format!("Invalid number for --timeout: {}", timeout_str),
                    }
                })?);
            }
            _ => {
                return Err(ParseError::GlobalOptionError {
                    option_name: name.to_string(),
                    error: "Unknown global option".to_string(),
                });
            }
        }
        Ok(())
    }

    fn finalize_task(&self, task_name: String, raw_args: HashMap<String, String>) -> Result<ParsedTask, ParseError> {
        let config = self.config.as_ref().ok_or_else(|| ParseError::NoConfigFound {
            searched_paths: vec![
                "otto.yml".to_string(),
                ".otto.yml".to_string(),
                "otto.yaml".to_string(),
                ".otto.yaml".to_string(),
                "Ottofile".to_string(),
                "OTTOFILE".to_string(),
            ],
        })?;

        let task_spec = config.tasks.get(&task_name).ok_or_else(|| {
            let task_names: Vec<String> = config.tasks.keys().cloned().collect();
            let suggestions = suggest_similar_task_names(&task_name, &task_names);

            ParseError::UnknownTask {
                name: task_name.clone(),
                suggestions,
            }
        })?;

        // Validate and convert arguments
        let mut validated_args = HashMap::new();

        for (arg_name, arg_value) in raw_args {
            let param_spec = task_spec.params.get(&arg_name).ok_or_else(|| {
                ParseError::InvalidArgument {
                    task_name: task_name.clone(),
                    arg_name: arg_name.clone(),
                    error: "Unknown argument for this task".to_string(),
                }
            })?;

            let validated_value = ArgumentValidator::validate_argument(&arg_value, param_spec)
                .map_err(|validation_error| ParseError::ValidationError {
                    task_name: task_name.clone(),
                    arg_name: arg_name.clone(),
                    validation_error,
                })?;

            validated_args.insert(arg_name, validated_value);
        }

        // Apply defaults
        ArgumentValidator::apply_defaults(&mut validated_args, task_spec)
            .map_err(|validation_error| ParseError::ValidationError {
                task_name: task_name.clone(),
                arg_name: "default".to_string(),
                validation_error,
            })?;

        // Check required arguments
        ArgumentValidator::validate_required_arguments(&validated_args, task_spec)
            .map_err(|missing_args| {
                // Return error for first missing argument
                ParseError::MissingRequiredArgument {
                    task_name: task_name.clone(),
                    arg_name: missing_args[0].clone(),
                }
            })?;

        Ok(ParsedTask {
            name: task_name,
            arguments: validated_args,
        })
    }

    fn get_default_tasks(&self) -> Vec<ParsedTask> {
        if let Some(ref config) = self.config {
            // Return the first task as default, or empty if no tasks
            if let Some((task_name, _)) = config.tasks.iter().next() {
                vec![ParsedTask {
                    name: task_name.clone(),
                    arguments: HashMap::new(),
                }]
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    }


}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::cfg::config::{ConfigSpec, OttoSpec, TaskSpec, ParamSpec};
    use crate::cfg::param::{ParamType, Nargs};
    use crate::cli2::types::ValidatedValue;

    #[test]
    fn test_empty_input() {
        let mut parser = NomParser::new(None).unwrap();
        let result = parser.parse("").unwrap();
        assert!(result.tasks.is_empty());
        assert!(!result.global_options.help);
        assert!(!result.global_options.version);
    }

    #[test]
    fn test_help_flag() {
        let mut parser = NomParser::new(None).unwrap();
        let result = parser.parse("--help").unwrap();
        assert!(result.global_options.help);
        assert!(result.tasks.is_empty());
    }

    #[test]
    fn test_version_flag() {
        let mut parser = NomParser::new(None).unwrap();
        let result = parser.parse("--version").unwrap();
        assert!(result.global_options.version);
        assert!(result.tasks.is_empty());
    }

    #[test]
    fn test_task_with_arguments() {
        // Create a test configuration
        let mut tasks = HashMap::new();
        let mut params = HashMap::new();
        params.insert("greeting".to_string(), ParamSpec {
            name: "greeting".to_string(),
            short: Some('g'),
            long: Some("greeting".to_string()),
            param_type: ParamType::OPT,
            dest: None,
            metavar: None,
            default: Some("hello".to_string()),
            constant: crate::cfg::param::Value::Empty,
            choices: vec!["hello".to_string(), "howdy".to_string()],
            nargs: Nargs::One,
            help: Some("Greeting to use".to_string()),
            value: crate::cfg::param::Value::Empty,
        });

        tasks.insert("hello".to_string(), TaskSpec {
            name: "hello".to_string(),
            help: Some("Say hello".to_string()),
            after: vec![],
            before: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params,
            action: "echo ${greeting}".to_string(),
            timeout: None,
        });

        let config = ConfigSpec {
            otto: OttoSpec {
                tasks: vec!["hello".to_string()],
                ..Default::default()
            },
            tasks,
        };

        let mut parser = NomParser::new(Some(config)).unwrap();
        let result = parser.parse("hello --greeting=howdy").unwrap();

        assert_eq!(result.tasks.len(), 1);
        assert_eq!(result.tasks[0].name, "hello");
        assert_eq!(result.tasks[0].arguments.len(), 1);
        assert!(result.tasks[0].arguments.contains_key("greeting"));

        if let ValidatedValue::String(value) = &result.tasks[0].arguments["greeting"] {
            assert_eq!(value, "howdy");
        } else {
            panic!("Expected string value for greeting");
        }
    }

    #[test]
    fn test_multiple_tasks_with_dependencies() {
        // Create a test configuration with dependencies
        let mut tasks = HashMap::new();

        tasks.insert("task1".to_string(), TaskSpec {
            name: "task1".to_string(),
            help: Some("First task".to_string()),
            after: vec!["task2".to_string()],
            before: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: HashMap::new(),
            action: "echo task1".to_string(),
            timeout: None,
        });

        tasks.insert("task2".to_string(), TaskSpec {
            name: "task2".to_string(),
            help: Some("Second task".to_string()),
            after: vec![],
            before: vec!["task1".to_string()],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: HashMap::new(),
            action: "echo task2".to_string(),
            timeout: None,
        });

        let config = ConfigSpec {
            otto: OttoSpec {
                tasks: vec!["task1".to_string()],
                ..Default::default()
            },
            tasks,
        };

        let mut parser = NomParser::new(Some(config)).unwrap();
        let result = parser.parse("task2").unwrap();

        assert_eq!(result.tasks.len(), 1);
        assert_eq!(result.tasks[0].name, "task2");
    }

    #[test]
    fn test_global_options() {
        let mut parser = NomParser::new(None).unwrap();
        let result = parser.parse("--jobs 4 --verbosity 2").unwrap();

        assert_eq!(result.global_options.jobs, Some(4));
        assert_eq!(result.global_options.verbosity, Some(2));
    }

    #[test]
    fn test_complex_command_parsing() {
        // Test the complex command from the user's original request
        let mut tasks = HashMap::new();

        // Build task with release parameter
        let mut build_params = HashMap::new();
        build_params.insert("release".to_string(), ParamSpec {
            name: "release".to_string(),
            short: Some('r'),
            long: Some("release".to_string()),
            param_type: ParamType::FLG,
            dest: None,
            metavar: None,
            default: Some("false".to_string()),
            constant: crate::cfg::param::Value::Empty,
            choices: vec![],
            nargs: Nargs::Zero,
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

        // Test task with verbose parameter
        let mut test_params = HashMap::new();
        test_params.insert("verbose".to_string(), ParamSpec {
            name: "verbose".to_string(),
            short: Some('v'),
            long: Some("verbose".to_string()),
            param_type: ParamType::FLG,
            dest: None,
            metavar: None,
            default: Some("false".to_string()),
            constant: crate::cfg::param::Value::Empty,
            choices: vec![],
            nargs: Nargs::Zero,
            help: Some("Verbose output".to_string()),
            value: crate::cfg::param::Value::Empty,
        });

        tasks.insert("test".to_string(), TaskSpec {
            name: "test".to_string(),
            help: Some("Run tests".to_string()),
            after: vec![],
            before: vec!["build".to_string()],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: test_params,
            action: "cargo test".to_string(),
            timeout: None,
        });

        // Deploy task with env parameter
        let mut deploy_params = HashMap::new();
        deploy_params.insert("env".to_string(), ParamSpec {
            name: "env".to_string(),
            short: Some('e'),
            long: Some("env".to_string()),
            param_type: ParamType::OPT,
            dest: None,
            metavar: None,
            default: Some("dev".to_string()),
            constant: crate::cfg::param::Value::Empty,
            choices: vec!["dev".to_string(), "staging".to_string(), "prod".to_string()],
            nargs: Nargs::One,
            help: Some("Environment to deploy to".to_string()),
            value: crate::cfg::param::Value::Empty,
        });

        tasks.insert("deploy".to_string(), TaskSpec {
            name: "deploy".to_string(),
            help: Some("Deploy the application".to_string()),
            after: vec![],
            before: vec!["test".to_string()],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: deploy_params,
            action: "deploy.sh".to_string(),
            timeout: None,
        });

        let config = ConfigSpec {
            otto: OttoSpec {
                tasks: vec!["build".to_string()],
                ..Default::default()
            },
            tasks,
        };

        let mut parser = NomParser::new(Some(config)).unwrap();

        // Test the complex command: build --release=true test --verbose deploy --env=staging
        let result = parser.parse("build --release=true test --verbose deploy --env=staging").unwrap();

        assert_eq!(result.tasks.len(), 3);

        // Check build task
        assert_eq!(result.tasks[0].name, "build");
        assert_eq!(result.tasks[0].arguments.len(), 1);
        if let ValidatedValue::Boolean(value) = &result.tasks[0].arguments["release"] {
            assert_eq!(*value, true);
        } else {
            panic!("Expected boolean value for release");
        }

        // Check test task
        assert_eq!(result.tasks[1].name, "test");
        assert_eq!(result.tasks[1].arguments.len(), 1);
        if let ValidatedValue::Boolean(value) = &result.tasks[1].arguments["verbose"] {
            assert_eq!(*value, true);
        } else {
            panic!("Expected boolean value for verbose");
        }

        // Check deploy task
        assert_eq!(result.tasks[2].name, "deploy");
        assert_eq!(result.tasks[2].arguments.len(), 1);
        if let ValidatedValue::String(value) = &result.tasks[2].arguments["env"] {
            assert_eq!(value, "staging");
        } else {
            panic!("Expected string value for env");
        }
    }

    #[test]
    fn test_default_values_applied() {
        // Test that default values are properly applied when not specified
        let mut tasks = HashMap::new();
        let mut params = HashMap::new();
        params.insert("greeting".to_string(), ParamSpec {
            name: "greeting".to_string(),
            short: Some('g'),
            long: Some("greeting".to_string()),
            param_type: ParamType::OPT,
            dest: None,
            metavar: None,
            default: Some("hello".to_string()),
            constant: crate::cfg::param::Value::Empty,
            choices: vec!["hello".to_string(), "howdy".to_string()],
            nargs: Nargs::One,
            help: Some("Greeting to use".to_string()),
            value: crate::cfg::param::Value::Empty,
        });

        tasks.insert("hello".to_string(), TaskSpec {
            name: "hello".to_string(),
            help: Some("Say hello".to_string()),
            after: vec![],
            before: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params,
            action: "echo ${greeting}".to_string(),
            timeout: None,
        });

        let config = ConfigSpec {
            otto: OttoSpec {
                tasks: vec!["hello".to_string()],
                ..Default::default()
            },
            tasks,
        };

        let mut parser = NomParser::new(Some(config)).unwrap();
        let result = parser.parse("hello").unwrap();

        assert_eq!(result.tasks.len(), 1);
        assert_eq!(result.tasks[0].name, "hello");
        assert_eq!(result.tasks[0].arguments.len(), 1);

        if let ValidatedValue::String(value) = &result.tasks[0].arguments["greeting"] {
            assert_eq!(value, "hello");
        } else {
            panic!("Expected string value for greeting");
        }
    }
}
