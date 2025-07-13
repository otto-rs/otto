use std::collections::HashMap;
use crate::cli::types::{ValidatedValue, ParsedTask, GlobalOptions};
use crate::cli::error::ParseError;
use crate::cfg::config::ConfigSpec;

use crate::cfg::param::ParamSpec;

pub fn suggest_similar_task_names(invalid_name: &str, valid_tasks: &[String]) -> Vec<String> {
    let mut suggestions: Vec<(String, usize)> = valid_tasks
        .iter()
        .map(|task| (task.clone(), levenshtein::levenshtein(invalid_name, task)))
        .filter(|(_, distance)| *distance <= 3) // Only suggest if distance <= 3
        .collect();

    suggestions.sort_by_key(|(_, distance)| *distance);
    suggestions.into_iter().take(3).map(|(name, _)| name).collect()
}

pub fn validate_global_options(
    global_options: &[crate::cli::types::GlobalOption],
) -> Result<GlobalOptions, ParseError> {
    let mut result = GlobalOptions::default();

    for option in global_options {
        match option.name.as_str() {
            "ottofile" => {
                if let Some(ref value) = option.value {
                    result.ottofile = Some(value.into());
                } else {
                    return Err(ParseError::MissingArgumentValue {
                        arg_name: "ottofile".to_string(),
                    });
                }
            }
            "api" => {
                if let Some(ref value) = option.value {
                    result.api = Some(value.clone());
                } else {
                    return Err(ParseError::MissingArgumentValue {
                        arg_name: "api".to_string(),
                    });
                }
            }
            "jobs" => {
                if let Some(ref value) = option.value {
                    result.jobs = Some(value.parse().map_err(|_| {
                        ParseError::InvalidArgumentValue {
                            arg_name: "jobs".to_string(),
                            value: value.clone(),
                            expected: "positive integer".to_string(),
                        }
                    })?);
                } else {
                    return Err(ParseError::MissingArgumentValue {
                        arg_name: "jobs".to_string(),
                    });
                }
            }
            "home" => {
                if let Some(ref value) = option.value {
                    result.home = Some(value.into());
                } else {
                    return Err(ParseError::MissingArgumentValue {
                        arg_name: "home".to_string(),
                    });
                }
            }
            "tasks" => {
                if let Some(ref value) = option.value {
                    result.tasks = Some(value.clone());
                } else {
                    return Err(ParseError::MissingArgumentValue {
                        arg_name: "tasks".to_string(),
                    });
                }
            }
            "verbosity" => {
                if let Some(ref value) = option.value {
                    result.verbosity = Some(value.parse().map_err(|_| {
                        ParseError::InvalidArgumentValue {
                            arg_name: "verbosity".to_string(),
                            value: value.clone(),
                            expected: "integer 0-9".to_string(),
                        }
                    })?);
                } else {
                    return Err(ParseError::MissingArgumentValue {
                        arg_name: "verbosity".to_string(),
                    });
                }
            }

            "help" => {
                result.help = true;
            }
            "version" => {
                result.version = true;
            }
            _ => {
                return Err(ParseError::UnknownGlobalOption {
                    name: option.name.clone(),
                });
            }
        }
    }

    Ok(result)
}

pub fn validate_task_invocation(
    task_invocation: &crate::cli::types::TaskInvocation,
    config: &ConfigSpec,
) -> Result<ParsedTask, ParseError> {
    let task_spec = config.tasks.get(&task_invocation.name).ok_or_else(|| {
        let task_names: Vec<String> = config.tasks.keys().cloned().collect();
        let suggestions = suggest_similar_task_names(&task_invocation.name, &task_names);

        ParseError::UnknownTask {
            name: task_invocation.name.clone(),
            suggestions,
        }
    })?;

    let mut resolved_args = HashMap::new();

    // Validate provided arguments, mapping short flags to long names first
    for arg in &task_invocation.arguments {
        // Map short flag to long name if needed
        let resolved_arg_name = if arg.name.len() == 1 {
            // This might be a short flag, find the corresponding long name
            let mut found_long_name = None;
            for (param_name, param_spec) in &task_spec.params {
                if param_spec.short == Some(arg.name.chars().next().unwrap()) {
                    found_long_name = Some(param_name.clone());
                    break;
                }
            }
            found_long_name.unwrap_or(arg.name.clone())
        } else {
            arg.name.clone()
        };

        let param_spec = task_spec.params.get(&resolved_arg_name).ok_or_else(|| {
            ParseError::UnknownTaskArgument {
                task_name: task_invocation.name.clone(),
                arg_name: arg.name.clone(),
            }
        })?;

        let validated_value = validate_argument_value(arg, param_spec, &task_invocation.name)?;
        resolved_args.insert(resolved_arg_name, validated_value);
    }

    // Apply defaults for missing parameters
    for (param_name, param_spec) in &task_spec.params {
        if !resolved_args.contains_key(param_name) {
            if let Some(ref default_value) = param_spec.default {
                let validated_default = validate_default_value(default_value, param_spec)?;
                resolved_args.insert(param_name.clone(), validated_default);
            }
        }
    }

    // Check for required parameters
    for (param_name, param_spec) in &task_spec.params {
        if param_spec.default.is_none() && !resolved_args.contains_key(param_name) {
            return Err(ParseError::ValidationError {
                task_name: task_invocation.name.clone(),
                arg_name: param_name.clone(),
                error: "Required parameter not provided".to_string(),
            });
        }
    }

    Ok(ParsedTask {
        name: task_invocation.name.clone(),
        arguments: resolved_args,
    })
}

fn validate_argument_value(
    arg: &crate::cli::types::TaskArgument,
    param_spec: &ParamSpec,
    task_name: &str,
) -> Result<ValidatedValue, ParseError> {
    let value = match &arg.value {
        Some(v) => v.clone(),
        None => {
            // This is a flag argument
            match param_spec.param_type {
                crate::cfg::param::ParamType::FLG => "true".to_string(),
                _ => return Err(ParseError::MissingArgumentValue {
                    arg_name: arg.name.clone(),
                }),
            }
        }
    };

    // Validate choices if specified
    if !param_spec.choices.is_empty() {
        if !param_spec.choices.contains(&value) {
            return Err(ParseError::ValidationError {
                task_name: task_name.to_string(),
                arg_name: arg.name.clone(),
                error: format!(
                    "Invalid choice '{}'. Valid choices are: {}",
                    value,
                    param_spec.choices.join(", ")
                ),
            });
        }
    }

    // Type validation based on default value or parameter type
    if let Some(ref default) = param_spec.default {
        if default == "true" || default == "false" {
            // Boolean parameter
            match value.to_lowercase().as_str() {
                "true" | "t" | "yes" | "y" | "1" => Ok(ValidatedValue::Boolean(true)),
                "false" | "f" | "no" | "n" | "0" => Ok(ValidatedValue::Boolean(false)),
                _ => Err(ParseError::ValidationError {
                    task_name: task_name.to_string(),
                    arg_name: arg.name.clone(),
                    error: format!("Invalid boolean value '{}'. Use true/false, yes/no, or 1/0", value),
                }),
            }
        } else if default.parse::<i64>().is_ok() {
            // Integer parameter
            value.parse::<i64>()
                .map(ValidatedValue::Integer)
                .map_err(|_| ParseError::ValidationError {
                    task_name: task_name.to_string(),
                    arg_name: arg.name.clone(),
                    error: format!("Invalid integer value '{}'", value),
                })
        } else if default.parse::<f64>().is_ok() {
            // Float parameter
            value.parse::<f64>()
                .map(ValidatedValue::Float)
                .map_err(|_| ParseError::ValidationError {
                    task_name: task_name.to_string(),
                    arg_name: arg.name.clone(),
                    error: format!("Invalid float value '{}'", value),
                })
        } else {
            // String parameter
            Ok(ValidatedValue::String(value))
        }
    } else {
        // No default, infer from parameter type
        match param_spec.param_type {
            crate::cfg::param::ParamType::FLG => Ok(ValidatedValue::Boolean(true)),
            _ => Ok(ValidatedValue::String(value)),
        }
    }
}

fn validate_default_value(
    default_value: &str,
    param_spec: &ParamSpec,
) -> Result<ValidatedValue, ParseError> {
    // For defaults, we don't validate choices (matches clap behavior)
    // But we can use param_type for better validation
    match param_spec.param_type {
        crate::cfg::param::ParamType::FLG => {
            // Flag parameters should be boolean
            if default_value == "true" || default_value == "false" {
                Ok(ValidatedValue::Boolean(default_value.parse().unwrap()))
            } else {
                Ok(ValidatedValue::Boolean(false)) // Default for flags
            }
        }
        _ => {
            // For other types, infer from the string value
            if default_value == "true" || default_value == "false" {
                Ok(ValidatedValue::Boolean(default_value.parse().unwrap()))
            } else if let Ok(int_val) = default_value.parse::<i64>() {
                Ok(ValidatedValue::Integer(int_val))
            } else if let Ok(float_val) = default_value.parse::<f64>() {
                Ok(ValidatedValue::Float(float_val))
            } else {
                Ok(ValidatedValue::String(default_value.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::types::GlobalOption;

    #[test]
    fn test_validate_global_options() {
        let options = vec![
            GlobalOption {
                name: "jobs".to_string(),
                value: Some("4".to_string()),
            },
            GlobalOption {
                name: "help".to_string(),
                value: None,
            },
        ];

        let result = validate_global_options(&options).unwrap();
        assert_eq!(result.jobs, Some(4));
        assert_eq!(result.help, true);
    }

    #[test]
    fn test_suggest_similar_task_names() {
        let tasks = vec!["hello".to_string(), "help".to_string(), "build".to_string()];
        let suggestions = suggest_similar_task_names("hell", &tasks);

        assert!(suggestions.contains(&"hello".to_string()));
        assert!(suggestions.contains(&"help".to_string()));
    }
}
