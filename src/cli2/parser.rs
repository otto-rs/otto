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

        let argument_regex = Regex::new(r"^--([a-zA-Z][a-zA-Z0-9_-]*)(=(.*))?$")
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
        match self.quick_parse(input) {
            Ok(result) => Ok(result),
            Err(_) => {
                // Fall back to full nom parsing
                self.full_parse(input)
            }
        }
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

            // Try to match global options first
            if let Some(captures) = self.argument_regex.captures(remaining) {
                let arg_name = captures.get(1).unwrap().as_str();
                let value = captures.get(3).map(|m| m.as_str().to_string());

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

                remaining = &remaining[captures.get(0).unwrap().end()..];
                continue;
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
            if remaining.starts_with("--help") || remaining.starts_with("-h") {
                tokens.push(Token::Help);
                remaining = if remaining.starts_with("--help") {
                    &remaining[6..]
                } else {
                    &remaining[2..]
                };
                continue;
            }

            if remaining.starts_with("--version") || remaining.starts_with("-V") {
                tokens.push(Token::Version);
                remaining = if remaining.starts_with("--version") {
                    &remaining[9..]
                } else {
                    &remaining[2..]
                };
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
                            // Look for next token as value
                            if i + 1 < tokens.len() {
                                if let Token::Unknown(next_val) = &tokens[i + 1] {
                                    i += 1; // Skip the next token
                                    next_val.clone()
                                } else {
                                    return Err(ParseError::InvalidArgument {
                                        task_name: current_task.unwrap_or_default(),
                                        arg_name: name.clone(),
                                        error: "Missing value for argument".to_string(),
                                    });
                                }
                            } else {
                                return Err(ParseError::InvalidArgument {
                                    task_name: current_task.unwrap_or_default(),
                                    arg_name: name.clone(),
                                    error: "Missing value for argument".to_string(),
                                });
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

    fn full_parse(&mut self, input: &str) -> Result<ParsedCommand, ParseError> {
        // Fallback to more complex nom parsing if needed
        // For now, just return an error
        Err(ParseError::ParsingError {
            input: input.to_string(),
            position: 0,
            expected: "Valid command format".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_empty_input() {
        let mut parser = NomParser::new(None).unwrap();
        let result = parser.parse("").unwrap();
        assert!(result.tasks.is_empty());
    }

    #[test]
    fn test_help_flag() {
        let mut parser = NomParser::new(None).unwrap();
        let result = parser.parse("--help").unwrap();
        assert!(result.global_options.help);
    }

    #[test]
    fn test_version_flag() {
        let mut parser = NomParser::new(None).unwrap();
        let result = parser.parse("--version").unwrap();
        assert!(result.global_options.version);
    }
}
