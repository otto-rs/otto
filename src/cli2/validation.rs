use std::collections::HashSet;
use crate::cli2::types::ValidatedValue;
use crate::cli2::error::{ValidationError, CollisionError};
use crate::cfg::config::ConfigSpec;
use crate::cfg::task::TaskSpec;
use crate::cfg::param::ParamSpec;

pub struct KeywordValidator {
    reserved_keywords: HashSet<String>,
    pub global_options: HashSet<String>,
}

impl KeywordValidator {
    pub fn new() -> Self {
        let mut reserved = HashSet::new();
        reserved.insert("help".to_string());
        reserved.insert("version".to_string());
        reserved.insert("--help".to_string());
        reserved.insert("-h".to_string());
        reserved.insert("--version".to_string());
        reserved.insert("-V".to_string());

        let mut global_opts = HashSet::new();
        global_opts.insert("ottofile".to_string());
        global_opts.insert("api".to_string());
        global_opts.insert("jobs".to_string());
        global_opts.insert("home".to_string());
        global_opts.insert("tasks".to_string());
        global_opts.insert("verbosity".to_string());
        global_opts.insert("timeout".to_string());

        Self {
            reserved_keywords: reserved,
            global_options: global_opts,
        }
    }

    pub fn validate_config(&self, config: &ConfigSpec) -> Result<(), Vec<CollisionError>> {
        let mut errors = Vec::new();
        let mut seen_tasks = HashSet::new();

        // Check for task name collisions and duplicates
        for task_name in config.tasks.keys() {
            // Check for duplicates
            if !seen_tasks.insert(task_name.clone()) {
                errors.push(CollisionError::DuplicateTaskName {
                    task_name: task_name.clone(),
                });
                continue;
            }

            // Check task name collisions
            if self.reserved_keywords.contains(task_name) {
                errors.push(CollisionError::TaskNameReserved {
                    task_name: task_name.clone(),
                });
            }

            if self.global_options.contains(task_name) {
                errors.push(CollisionError::TaskNameConflictsWithGlobalOption {
                    task_name: task_name.clone(),
                });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn validate_task_spec(&self, task_name: &str, task_spec: &TaskSpec) -> Result<(), Vec<CollisionError>> {
        let mut errors = Vec::new();
        let mut seen_args = HashSet::new();

        // Check argument name collisions within task
        for arg_name in task_spec.params.keys() {
            // Check for duplicates
            if !seen_args.insert(arg_name.clone()) {
                errors.push(CollisionError::DuplicateArgumentName {
                    task_name: task_name.to_string(),
                    arg_name: arg_name.clone(),
                });
                continue;
            }

            // Check reserved names
            if self.reserved_keywords.contains(arg_name) {
                errors.push(CollisionError::ArgumentNameReserved {
                    task_name: task_name.to_string(),
                    arg_name: arg_name.clone(),
                });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

pub struct ArgumentValidator;

impl ArgumentValidator {
    pub fn validate_argument(value: &str, param_spec: &ParamSpec) -> Result<ValidatedValue, ValidationError> {
        // For now, just return string values - we can extend this based on param types
        if !param_spec.choices.is_empty() {
            if param_spec.choices.contains(&value.to_string()) {
                Ok(ValidatedValue::String(value.to_string()))
            } else {
                Err(ValidationError::InvalidChoice {
                    argument: param_spec.name.clone(),
                    value: value.to_string(),
                    choices: param_spec.choices.clone(),
                })
            }
        } else {
            // Try to infer type from default value
            if let Some(ref default) = param_spec.default {
                if default == "true" || default == "false" {
                    // Boolean parameter
                    match value.to_lowercase().as_str() {
                        "true" | "t" | "yes" | "y" | "1" => Ok(ValidatedValue::Boolean(true)),
                        "false" | "f" | "no" | "n" | "0" => Ok(ValidatedValue::Boolean(false)),
                        _ => Err(ValidationError::InvalidType {
                            expected: "boolean (true/false, yes/no, 1/0)".to_string(),
                            got: value.to_string(),
                            argument: param_spec.name.clone(),
                        })
                    }
                } else if default.parse::<i64>().is_ok() {
                    // Integer parameter
                    value.parse::<i64>()
                        .map(ValidatedValue::Integer)
                        .map_err(|_| ValidationError::InvalidType {
                            expected: "integer".to_string(),
                            got: value.to_string(),
                            argument: param_spec.name.clone(),
                        })
                } else if default.parse::<f64>().is_ok() {
                    // Float parameter
                    value.parse::<f64>()
                        .map(ValidatedValue::Float)
                        .map_err(|_| ValidationError::InvalidType {
                            expected: "float".to_string(),
                            got: value.to_string(),
                            argument: param_spec.name.clone(),
                        })
                } else {
                    // String parameter
                    Ok(ValidatedValue::String(value.to_string()))
                }
            } else {
                // No default, assume string
                Ok(ValidatedValue::String(value.to_string()))
            }
        }
    }

    pub fn validate_required_arguments(
        provided_args: &std::collections::HashMap<String, ValidatedValue>,
        task_spec: &TaskSpec,
    ) -> Result<(), Vec<String>> {
        let mut missing = Vec::new();

        for (param_name, param_spec) in &task_spec.params {
            // Check if this parameter is required (has no default)
            if param_spec.default.is_none() && !provided_args.contains_key(param_name) {
                missing.push(param_name.clone());
            }
        }

        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }

    pub fn apply_defaults(
        provided_args: &mut std::collections::HashMap<String, ValidatedValue>,
        task_spec: &TaskSpec,
    ) -> Result<(), ValidationError> {
        for (param_name, param_spec) in &task_spec.params {
            if !provided_args.contains_key(param_name) {
                if let Some(ref default_value) = param_spec.default {
                    // For defaults, we skip choice validation to match clap behavior
                    // Clap allows defaults that aren't in the choices list
                    let validated = if !param_spec.choices.is_empty() {
                        // Skip choice validation for defaults
                        Ok(ValidatedValue::String(default_value.clone()))
                    } else {
                        Self::validate_argument(default_value, param_spec)
                    }?;
                    provided_args.insert(param_name.clone(), validated);
                }
            }
        }
        Ok(())
    }
}

pub fn suggest_similar_task_names(invalid_name: &str, valid_tasks: &[String]) -> Vec<String> {
    let mut suggestions: Vec<(String, usize)> = valid_tasks
        .iter()
        .map(|task| (task.clone(), levenshtein::levenshtein(invalid_name, task)))
        .filter(|(_, distance)| *distance <= 3) // Only suggest if distance <= 3
        .collect();

    suggestions.sort_by_key(|(_, distance)| *distance);
    suggestions.into_iter().take(3).map(|(name, _)| name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_keyword_validator() {
        let validator = KeywordValidator::new();

        // Test reserved keyword detection
        assert!(validator.reserved_keywords.contains("help"));
        assert!(validator.global_options.contains("jobs"));
    }

    #[test]
    fn test_task_name_suggestions() {
        let tasks = vec!["hello".to_string(), "help".to_string(), "build".to_string()];
        let suggestions = suggest_similar_task_names("hell", &tasks);

        assert!(suggestions.contains(&"hello".to_string()));
        assert!(suggestions.contains(&"help".to_string()));
    }
}
