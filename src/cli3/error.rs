use std::fmt;
use nom::error::{ErrorKind, ParseError as NomParseError, ContextError};
use colored::Colorize;

#[derive(Debug, Clone)]
pub enum ParseError {
    // nom-specific errors
    NomError {
        input: String,
        position: usize,
        kind: ErrorKind,
        context: Vec<String>,
    },
    
    // Semantic errors
    UnknownTask {
        name: String,
        suggestions: Vec<String>,
    },
    
    UnknownGlobalOption {
        name: String,
    },
    
    UnknownTaskArgument {
        task_name: String,
        arg_name: String,
    },
    
    MissingArgumentValue {
        arg_name: String,
    },
    
    InvalidArgumentValue {
        arg_name: String,
        value: String,
        expected: String,
    },
    
    // Validation errors
    ValidationError {
        task_name: String,
        arg_name: String,
        error: String,
    },
    
    // Config errors
    NoConfigFound {
        searched_paths: Vec<String>,
    },
    
    // Input errors
    UnconsumedInput {
        remaining: String,
    },
    
    IncompleteInput,
}

impl NomParseError<&str> for ParseError {
    fn from_error_kind(input: &str, kind: ErrorKind) -> Self {
        ParseError::NomError {
            input: input.to_string(),
            position: 0,
            kind,
            context: vec![],
        }
    }
    
    fn append(input: &str, kind: ErrorKind, mut other: Self) -> Self {
        match &mut other {
            ParseError::NomError { context, .. } => {
                context.push(format!("{:?} at '{}'", kind, input));
                other
            }
            _ => other,
        }
    }
}

impl ContextError<&str> for ParseError {
    fn add_context(input: &str, ctx: &'static str, mut other: Self) -> Self {
        match &mut other {
            ParseError::NomError { context, .. } => {
                context.push(format!("{} at '{}'", ctx, input));
                other
            }
            _ => other,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::NomError { input, position, kind, context } => {
                write!(f, "{}: Parse error at position {}", "error".red().bold(), position)?;
                if !context.is_empty() {
                    write!(f, "\nContext: {}", context.join(" -> "))?;
                }
                write!(f, "\nInput: {}", input)?;
                write!(f, "\nKind: {:?}", kind)
            }
            
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
            
            ParseError::UnknownGlobalOption { name } => {
                write!(
                    f,
                    "{}: Unknown global option '{}'\n\nFor more information try {}",
                    "error".red().bold(),
                    name.yellow(),
                    "--help".cyan()
                )
            }
            
            ParseError::UnknownTaskArgument { task_name, arg_name } => {
                write!(
                    f,
                    "{}: Unknown argument '{}' for task '{}'\n\nFor more information try {}",
                    "error".red().bold(),
                    arg_name.yellow(),
                    task_name.blue(),
                    format!("otto {} --help", task_name).cyan()
                )
            }
            
            ParseError::MissingArgumentValue { arg_name } => {
                write!(
                    f,
                    "{}: Missing value for argument '{}'",
                    "error".red().bold(),
                    arg_name.yellow()
                )
            }
            
            ParseError::InvalidArgumentValue { arg_name, value, expected } => {
                write!(
                    f,
                    "{}: Invalid value '{}' for argument '{}'\nExpected: {}",
                    "error".red().bold(),
                    value.red(),
                    arg_name.yellow(),
                    expected.green()
                )
            }
            
            ParseError::ValidationError { task_name, arg_name, error } => {
                write!(
                    f,
                    "{}: Validation failed for argument '{}' in task '{}'\n\n{}\n\nFor more information try {}",
                    "error".red().bold(),
                    arg_name.yellow(),
                    task_name.blue(),
                    error,
                    format!("otto {} --help", task_name).cyan()
                )
            }
            
            ParseError::NoConfigFound { searched_paths } => {
                writeln!(f, "{}: No ottofile found in this directory or any parent directory!", "ERROR".red().bold())?;
                writeln!(f, "Otto looks for one of the following files in the current or parent directories:")?;
                writeln!(f)?;
                for path in searched_paths {
                    writeln!(f, "  - {}", path.green())?;
                }
                write!(f, "\nTo get started, create an otto.yml file in your project root.")
            }
            
            ParseError::UnconsumedInput { remaining } => {
                write!(
                    f,
                    "{}: Unexpected input: '{}'",
                    "error".red().bold(),
                    remaining
                )
            }
            
            ParseError::IncompleteInput => {
                write!(f, "{}: Incomplete input", "error".red().bold())
            }
        }
    }
}

impl std::error::Error for ParseError {} 