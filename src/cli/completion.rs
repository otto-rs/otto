use crate::cli::types::{ParseState, PartialParseResult};
use crate::cli::validation::KeywordValidator;
use crate::cfg::config::ConfigSpec;

#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub description: String,
    pub completion_type: CompletionType,
}

#[derive(Debug, Clone)]
pub enum CompletionType {
    TaskName,
    GlobalOption,
    TaskArgument,
    ArgumentValue,
    File,
    Directory,
}

pub struct CompletionGenerator {
    config: Option<ConfigSpec>,
    #[allow(dead_code)]
    keyword_validator: KeywordValidator,
}

impl CompletionGenerator {
    pub fn new(config: Option<ConfigSpec>) -> Self {
        Self {
            config,
            keyword_validator: KeywordValidator::new(),
        }
    }

    pub fn generate_completions(&self, partial_input: &str) -> Vec<Completion> {
        let mut completions = Vec::new();

        // Parse what we have so far
        let parsed_partial = self.parse_partial(partial_input);

        match parsed_partial.state {
            ParseState::ExpectingGlobalOption => {
                completions.extend(self.global_option_completions());
            }
            ParseState::ExpectingTaskName => {
                completions.extend(self.task_name_completions(partial_input));
            }
            ParseState::ExpectingTaskArgument { ref task_name } => {
                completions.extend(self.task_argument_completions(task_name, partial_input));
            }
            ParseState::ExpectingArgumentValue { ref task_name, ref arg_name } => {
                completions.extend(self.argument_value_completions(task_name, arg_name));
            }
        }

        completions
    }

    fn parse_partial(&self, input: &str) -> PartialParseResult {
        let tokens: Vec<&str> = input.split_whitespace().collect();

        if tokens.is_empty() {
            return PartialParseResult {
                state: ParseState::ExpectingGlobalOption,
                tokens: vec![],
            };
        }

        let last_token = tokens.last().unwrap();

        // Simple state detection for completion
        if last_token.starts_with("--") {
            // Check if this could be a task argument
            if let Some(current_task) = self.find_current_task(&tokens) {
                PartialParseResult {
                    state: ParseState::ExpectingTaskArgument {
                        task_name: current_task,
                    },
                    tokens: tokens.iter().map(|s| s.to_string()).collect(),
                }
            } else {
                PartialParseResult {
                    state: ParseState::ExpectingGlobalOption,
                    tokens: tokens.iter().map(|s| s.to_string()).collect(),
                }
            }
        } else if self.is_task_name(last_token) {
            PartialParseResult {
                state: ParseState::ExpectingTaskName,
                tokens: tokens.iter().map(|s| s.to_string()).collect(),
            }
        } else {
            PartialParseResult {
                state: ParseState::ExpectingTaskName,
                tokens: tokens.iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    fn find_current_task(&self, tokens: &[&str]) -> Option<String> {
        if let Some(ref config) = self.config {
            // Find the last task name in the tokens
            for token in tokens.iter().rev() {
                if config.tasks.contains_key(*token) {
                    return Some(token.to_string());
                }
            }
        }
        None
    }

    fn is_task_name(&self, token: &str) -> bool {
        if let Some(ref config) = self.config {
            config.tasks.contains_key(token)
        } else {
            false
        }
    }

    fn global_option_completions(&self) -> Vec<Completion> {
        vec![
            Completion {
                text: "--ottofile".to_string(),
                description: "path to the ottofile".to_string(),
                completion_type: CompletionType::GlobalOption,
            },
            Completion {
                text: "--api".to_string(),
                description: "api url".to_string(),
                completion_type: CompletionType::GlobalOption,
            },
            Completion {
                text: "--jobs".to_string(),
                description: "number of jobs to run in parallel".to_string(),
                completion_type: CompletionType::GlobalOption,
            },
            Completion {
                text: "--home".to_string(),
                description: "path to the Otto home directory".to_string(),
                completion_type: CompletionType::GlobalOption,
            },
            Completion {
                text: "--tasks".to_string(),
                description: "comma separated list of tasks to run".to_string(),
                completion_type: CompletionType::GlobalOption,
            },
            Completion {
                text: "--verbosity".to_string(),
                description: "verbosity level".to_string(),
                completion_type: CompletionType::GlobalOption,
            },
            Completion {
                text: "--timeout".to_string(),
                description: "global timeout in seconds".to_string(),
                completion_type: CompletionType::GlobalOption,
            },
            Completion {
                text: "--help".to_string(),
                description: "Print help".to_string(),
                completion_type: CompletionType::GlobalOption,
            },
            Completion {
                text: "--version".to_string(),
                description: "Print version".to_string(),
                completion_type: CompletionType::GlobalOption,
            },
        ]
    }

    fn task_name_completions(&self, partial_input: &str) -> Vec<Completion> {
        if let Some(ref config) = self.config {
            let last_word = partial_input.split_whitespace().last().unwrap_or("");

            config.tasks.iter()
                .filter(|(name, _)| name.starts_with(last_word))
                .map(|(name, spec)| Completion {
                    text: name.clone(),
                    description: spec.help.clone().unwrap_or_else(|| "Task".to_string()),
                    completion_type: CompletionType::TaskName,
                })
                .collect()
        } else {
            vec![]
        }
    }

    fn task_argument_completions(&self, task_name: &str, partial_input: &str) -> Vec<Completion> {
        if let Some(ref config) = self.config {
            if let Some(task_spec) = config.tasks.get(task_name) {
                let last_word = partial_input.split_whitespace().last().unwrap_or("");

                task_spec.params.iter()
                    .filter(|(name, _)| format!("--{}", name).starts_with(last_word))
                    .map(|(name, spec)| Completion {
                        text: format!("--{}", name),
                        description: spec.help.clone().unwrap_or_else(|| format!("Parameter for {}", task_name)),
                        completion_type: CompletionType::TaskArgument,
                    })
                    .collect()
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    }

    fn argument_value_completions(&self, task_name: &str, arg_name: &str) -> Vec<Completion> {
        if let Some(ref config) = self.config {
            if let Some(task_spec) = config.tasks.get(task_name) {
                if let Some(param_spec) = task_spec.params.get(arg_name) {
                    if !param_spec.choices.is_empty() {
                        return param_spec.choices.iter()
                            .map(|choice| Completion {
                                text: choice.clone(),
                                description: format!("Option for --{}", arg_name),
                                completion_type: CompletionType::ArgumentValue,
                            })
                            .collect();
                    }

                    // Type-specific completions based on default value
                    if let Some(ref default) = param_spec.default {
                        if default == "true" || default == "false" {
                            vec![
                                Completion {
                                    text: "true".to_string(),
                                    description: "Boolean true".to_string(),
                                    completion_type: CompletionType::ArgumentValue,
                                },
                                Completion {
                                    text: "false".to_string(),
                                    description: "Boolean false".to_string(),
                                    completion_type: CompletionType::ArgumentValue,
                                },
                            ]
                        } else {
                            vec![]
                        }
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    }
}

// Shell-specific completion generators
pub struct BashCompletionGenerator;
pub struct ZshCompletionGenerator;
pub struct FishCompletionGenerator;

impl BashCompletionGenerator {
    pub fn generate_script(completions: &[Completion]) -> String {
        let mut script = String::from("_otto_completion() {\n");
        script.push_str("    local cur prev opts\n");
        script.push_str("    COMPREPLY=()\n");
        script.push_str("    cur=\"${COMP_WORDS[COMP_CWORD]}\"\n");
        script.push_str("    prev=\"${COMP_WORDS[COMP_CWORD-1]}\"\n");
        script.push_str("    opts=\"");

        for completion in completions {
            script.push_str(&completion.text);
            script.push(' ');
        }

        script.push_str("\"\n");
        script.push_str("    COMPREPLY=( $(compgen -W \"${opts}\" -- ${cur}) )\n");
        script.push_str("    return 0\n");
        script.push_str("}\n");
        script.push_str("complete -F _otto_completion otto\n");

        script
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn test_global_option_completions() {
        let generator = CompletionGenerator::new(None);
        let completions = generator.global_option_completions();

        assert!(!completions.is_empty());
        assert!(completions.iter().any(|c| c.text == "--help"));
        assert!(completions.iter().any(|c| c.text == "--version"));
    }

    #[test]
    fn test_completion_generation() {
        let generator = CompletionGenerator::new(None);
        let completions = generator.generate_completions("--");

        assert!(!completions.is_empty());
    }
}
