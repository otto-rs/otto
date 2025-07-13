use crate::cli::combinators::parse_command_line;
use crate::cli::types::{ParsedCommand, ParsedTask, GlobalOptions, RawParsedCommand};
use crate::cli::error::ParseError;
use crate::cli::validation::{validate_global_options, validate_task_invocation};
use crate::cfg::config::ConfigSpec;

pub struct NomParser {
    config: Option<ConfigSpec>,
}

impl NomParser {
    pub fn new(config: Option<ConfigSpec>) -> Result<Self, ParseError> {
        Ok(Self { config })
    }

    pub fn parse(&mut self, input: &str) -> Result<ParsedCommand, ParseError> {
        let input = input.trim();

        // Handle empty input
        if input.is_empty() {
            return Ok(ParsedCommand {
                global_options: GlobalOptions::default(),
                tasks: self.get_default_tasks(),
            });
        }

        // Parse using nom combinators
        let raw_command = match parse_command_line(input) {
            Ok((remaining, parsed)) => {
                if !remaining.trim().is_empty() {
                    return Err(ParseError::UnconsumedInput {
                        remaining: remaining.to_string(),
                    });
                }
                parsed
            }
            Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => return Err(e),
            Err(nom::Err::Incomplete(_)) => return Err(ParseError::IncompleteInput),
        };

        // Validate and convert to final types
        self.validate_and_convert(raw_command)
    }

    fn validate_and_convert(&self, raw: RawParsedCommand) -> Result<ParsedCommand, ParseError> {
        // Validate global options
        let global_options = validate_global_options(&raw.global_options)?;

        // Handle help/version flags early
        if global_options.help || global_options.version {
            return Ok(ParsedCommand {
                global_options,
                tasks: vec![],
            });
        }

        // Validate tasks
        let mut validated_tasks = Vec::new();

        if raw.tasks.is_empty() {
            // No tasks specified, use defaults
            validated_tasks = self.get_default_tasks();
        } else {
            // Validate each task
            for task_invocation in &raw.tasks {
                if let Some(ref config) = self.config {
                    let validated_task = validate_task_invocation(task_invocation, config)?;
                    validated_tasks.push(validated_task);
                } else {
                    return Err(ParseError::NoConfigFound {
                        searched_paths: vec![
                            "otto.yml".to_string(),
                            ".otto.yml".to_string(),
                            "otto.yaml".to_string(),
                            ".otto.yaml".to_string(),
                            "Ottofile".to_string(),
                            "OTTOFILE".to_string(),
                        ],
                    });
                }
            }
        }

        Ok(ParsedCommand {
            global_options,
            tasks: validated_tasks,
        })
    }

    fn get_default_tasks(&self) -> Vec<ParsedTask> {
        if let Some(ref config) = self.config {
            config.otto.tasks
                .iter()
                .filter_map(|task_name| {
                    if config.tasks.contains_key(task_name) {
                        Some(ParsedTask {
                            name: task_name.clone(),
                            arguments: std::collections::HashMap::new(),
                        })
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::cfg::config::{ConfigSpec, OttoSpec};
    use crate::cfg::task::TaskSpec;
    use crate::cfg::param::{ParamSpec, ParamType, Nargs};
    use crate::cli::types::ValidatedValue;

    fn create_test_config() -> ConfigSpec {
        let mut tasks = HashMap::new();

        // Hello task with greeting parameter
        let mut hello_params = HashMap::new();
        hello_params.insert("greeting".to_string(), ParamSpec {
            name: "greeting".to_string(),
            short: Some('g'),
            long: Some("greeting".to_string()),
            param_type: ParamType::OPT,
            dest: None,
            metavar: None,
            default: Some("hello".to_string()),
            constant: crate::cfg::param::Value::Empty,
            choices: vec![],
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
            params: hello_params,
            action: "echo \"$greeting World!\"".to_string(),
        });

        // World task with name parameter
        let mut world_params = HashMap::new();
        world_params.insert("name".to_string(), ParamSpec {
            name: "name".to_string(),
            short: Some('n'),
            long: Some("name".to_string()),
            param_type: ParamType::OPT,
            dest: None,
            metavar: None,
            default: Some("world".to_string()),
            constant: crate::cfg::param::Value::Empty,
            choices: vec![],
            nargs: Nargs::One,
            help: Some("Name to use".to_string()),
            value: crate::cfg::param::Value::Empty,
        });

        tasks.insert("world".to_string(), TaskSpec {
            name: "world".to_string(),
            help: Some("Say world".to_string()),
            after: vec![],
            before: vec!["hello".to_string()],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: world_params,
            action: "echo \"$name\"".to_string(),
            timeout: None,
        });

        // Punch task (default)
        tasks.insert("punch".to_string(), TaskSpec {
            name: "punch".to_string(),
            help: Some("Punch task".to_string()),
            after: vec![],
            before: vec![],
            input: vec![],
            output: vec![],
            envs: HashMap::new(),
            params: HashMap::new(),
            action: "echo \"donkey\"".to_string(),
            timeout: None,
        });

        ConfigSpec {
            otto: OttoSpec {
                tasks: vec!["punch".to_string()],
                ..Default::default()
            },
            tasks,
        }
    }

    #[test]
    fn test_empty_input() {
        let config = create_test_config();
        let mut parser = NomParser::new(Some(config)).unwrap();
        let result = parser.parse("").unwrap();

        assert_eq!(result.tasks.len(), 1);
        assert_eq!(result.tasks[0].name, "punch");
    }

    #[test]
    fn test_help_flag() {
        let mut parser = NomParser::new(None).unwrap();
        let result = parser.parse("--help").unwrap();

        assert!(result.global_options.help);
        assert!(result.tasks.is_empty());
    }

    #[test]
    fn test_simple_task() {
        let config = create_test_config();
        let mut parser = NomParser::new(Some(config)).unwrap();
        let result = parser.parse("hello").unwrap();

        assert_eq!(result.tasks.len(), 1);
        assert_eq!(result.tasks[0].name, "hello");
        // Should have default greeting
        assert!(result.tasks[0].arguments.contains_key("greeting"));
        if let ValidatedValue::String(greeting) = &result.tasks[0].arguments["greeting"] {
            assert_eq!(greeting, "hello");
        }
    }

    #[test]
    fn test_task_with_short_flag() {
        let config = create_test_config();
        let mut parser = NomParser::new(Some(config)).unwrap();
        let result = parser.parse("hello -g howdy").unwrap();

        assert_eq!(result.tasks.len(), 1);
        assert_eq!(result.tasks[0].name, "hello");
        assert!(result.tasks[0].arguments.contains_key("greeting"));
        if let ValidatedValue::String(greeting) = &result.tasks[0].arguments["greeting"] {
            assert_eq!(greeting, "howdy");
        }
    }

    #[test]
    fn test_multiple_tasks() {
        let config = create_test_config();
        let mut parser = NomParser::new(Some(config)).unwrap();
        let result = parser.parse("hello -g howdy world -n mundo").unwrap();

        assert_eq!(result.tasks.len(), 2);
        assert_eq!(result.tasks[0].name, "hello");
        assert_eq!(result.tasks[1].name, "world");

        if let ValidatedValue::String(greeting) = &result.tasks[0].arguments["greeting"] {
            assert_eq!(greeting, "howdy");
        }
        if let ValidatedValue::String(name) = &result.tasks[1].arguments["name"] {
            assert_eq!(name, "mundo");
        }
    }

    #[test]
    fn test_global_options() {
        let config = create_test_config();
        let mut parser = NomParser::new(Some(config)).unwrap();
        let result = parser.parse("--ottofile examples/ex1 hello").unwrap();

        assert!(result.global_options.ottofile.is_some());
        assert_eq!(result.global_options.ottofile.unwrap().to_string_lossy(), "examples/ex1");
        assert_eq!(result.tasks.len(), 1);
        assert_eq!(result.tasks[0].name, "hello");
    }

    #[test]
    fn test_unknown_task() {
        let config = create_test_config();
        let mut parser = NomParser::new(Some(config)).unwrap();
        let result = parser.parse("hell");  // Close to "hello"

        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::UnknownTask { name, suggestions } => {
                assert_eq!(name, "hell");
                assert!(!suggestions.is_empty());
                assert!(suggestions.contains(&"hello".to_string()));
            }
            _ => panic!("Expected UnknownTask error"),
        }
    }

    #[test]
    fn test_world_task_runs_hello_dependency() {
        let config = create_test_config();
        let mut parser = NomParser::new(Some(config)).unwrap();
        let result = parser.parse("world").unwrap();

        // Note: This test just verifies parsing, not dependency resolution
        // Dependency resolution happens in the executor
        assert_eq!(result.tasks.len(), 1);
        assert_eq!(result.tasks[0].name, "world");
    }
}
