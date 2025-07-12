use std::fmt;
use colored::Colorize;

#[derive(Debug, Clone)]
pub enum ParseError {
    UnknownTask {
        name: String,
        suggestions: Vec<String>,
    },
    InvalidArgument {
        task_name: String,
        arg_name: String,
        error: String,
    },
    MissingRequiredArgument {
        task_name: String,
        arg_name: String,
    },
    ValidationError {
        task_name: String,
        arg_name: String,
        validation_error: ValidationError,
    },
    CollisionError {
        errors: Vec<CollisionError>,
    },
    GlobalOptionError {
        option_name: String,
        error: String,
    },
    ParsingError {
        input: String,
        position: usize,
        expected: String,
    },
    NoConfigFound {
        searched_paths: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub enum ValidationError {
    InvalidType {
        expected: String,
        got: String,
        argument: String,
    },
    InvalidChoice {
        argument: String,
        value: String,
        choices: Vec<String>,
    },
    InvalidPath {
        argument: String,
        path: String,
        error: String,
    },
    InvalidUrl {
        argument: String,
        url: String,
        error: String,
    },
    OutOfRange {
        argument: String,
        value: String,
        min: Option<i64>,
        max: Option<i64>,
    },
}

#[derive(Debug, Clone)]
pub enum CollisionError {
    TaskNameReserved {
        task_name: String,
    },
    TaskNameConflictsWithGlobalOption {
        task_name: String,
    },
    ArgumentNameReserved {
        task_name: String,
        arg_name: String,
    },
    DuplicateTaskName {
        task_name: String,
    },
    DuplicateArgumentName {
        task_name: String,
        arg_name: String,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnknownTask { name, suggestions } => {
                write!(f, "{}: The task '{}' wasn't found", "error".red().bold(), name)?;

                if !suggestions.is_empty() {
                    writeln!(f)?;
                    writeln!(f)?;
                    if suggestions.len() == 1 {
                        write!(f, "Did you mean '{}'?", suggestions[0].green())?;
                    } else {
                        writeln!(f, "Did you mean one of these?")?;
                        for suggestion in suggestions {
                            writeln!(f, "    {}", suggestion.green())?;
                        }
                    }
                }

                writeln!(f)?;
                write!(f, "For more information try {}", "--help".cyan())
            }

            ParseError::InvalidArgument { task_name, arg_name, error } => {
                write!(
                    f,
                    "{}: Invalid argument '{}' for task '{}'\n\n{}\n\nFor more information try {}",
                    "error".red().bold(),
                    arg_name.yellow(),
                    task_name.blue(),
                    error,
                    format!("otto {} --help", task_name).cyan()
                )
            }

            ParseError::MissingRequiredArgument { task_name, arg_name } => {
                write!(
                    f,
                    "{}: The following required argument was not provided for task '{}':\n    {}\n\nFor more information try {}",
                    "error".red().bold(),
                    task_name.blue(),
                    format!("--{}", arg_name).yellow(),
                    format!("otto {} --help", task_name).cyan()
                )
            }

            ParseError::ValidationError { task_name, arg_name, validation_error } => {
                write!(
                    f,
                    "{}: Validation failed for argument '{}' in task '{}'\n\n{}\n\nFor more information try {}",
                    "error".red().bold(),
                    arg_name.yellow(),
                    task_name.blue(),
                    validation_error,
                    format!("otto {} --help", task_name).cyan()
                )
            }

            ParseError::CollisionError { errors } => {
                writeln!(f, "{}: Configuration conflicts detected:", "error".red().bold())?;
                writeln!(f)?;

                for error in errors {
                    writeln!(f, "  {}", error)?;
                }

                write!(f, "\nPlease rename the conflicting items in your otto.yml file.")
            }

            ParseError::GlobalOptionError { option_name, error } => {
                write!(
                    f,
                    "{}: Invalid global option '{}'\n\n{}\n\nFor more information try {}",
                    "error".red().bold(),
                    option_name.yellow(),
                    error,
                    "--help".cyan()
                )
            }

            ParseError::ParsingError { input, position, expected } => {
                write!(
                    f,
                    "{}: Failed to parse command line\n\nInput: {}\nPosition: {}\nExpected: {}",
                    "error".red().bold(),
                    input,
                    position,
                    expected
                )
            }

            ParseError::NoConfigFound { searched_paths } => {
                writeln!(f, "{}: No ottofile found in this directory or any parent directory!", "ERROR".red().bold())?;
                writeln!(f, "Otto looks for one of the following files in the current or parent directories:")?;
                writeln!(f)?;
                writeln!(f, "To get started, create an otto.yml file in your project root.")?;
                for path in searched_paths {
                    writeln!(f, "  - {}", path.green())?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::InvalidType { expected, got, .. } => {
                write!(f, "Invalid value '{}'\n\nExpected: {}", got.red(), expected.green())
            }
            ValidationError::InvalidChoice { value, choices, .. } => {
                write!(
                    f,
                    "Invalid value '{}'\n\nPossible values: [{}]",
                    value.red(),
                    choices.iter().map(|c| c.green().to_string()).collect::<Vec<_>>().join(", ")
                )
            }
            ValidationError::InvalidPath { path, error, .. } => {
                write!(f, "Invalid path '{}': {}", path.red(), error)
            }
            ValidationError::InvalidUrl { url, error, .. } => {
                write!(f, "Invalid URL '{}': {}", url.red(), error)
            }
            ValidationError::OutOfRange { value, min, max, .. } => {
                write!(f, "Value '{}' is out of range", value.red())?;
                match (min, max) {
                    (Some(min), Some(max)) => write!(f, " (expected: {} to {})", min, max),
                    (Some(min), None) => write!(f, " (expected: >= {})", min),
                    (None, Some(max)) => write!(f, " (expected: <= {})", max),
                    (None, None) => Ok(()),
                }
            }
        }
    }
}

impl fmt::Display for CollisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CollisionError::TaskNameReserved { task_name } => {
                write!(f, "Task name '{}' is reserved", task_name.yellow())
            }
            CollisionError::TaskNameConflictsWithGlobalOption { task_name } => {
                write!(f, "Task name '{}' conflicts with global option", task_name.yellow())
            }
            CollisionError::ArgumentNameReserved { task_name, arg_name } => {
                write!(
                    f,
                    "Argument name '{}' in task '{}' is reserved",
                    arg_name.yellow(),
                    task_name.blue()
                )
            }
            CollisionError::DuplicateTaskName { task_name } => {
                write!(f, "Duplicate task name '{}'", task_name.yellow())
            }
            CollisionError::DuplicateArgumentName { task_name, arg_name } => {
                write!(
                    f,
                    "Duplicate argument name '{}' in task '{}'",
                    arg_name.yellow(),
                    task_name.blue()
                )
            }
        }
    }
}

impl std::error::Error for ParseError {}
impl std::error::Error for ValidationError {}
impl std::error::Error for CollisionError {}
