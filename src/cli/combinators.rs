use nom::{
    IResult, Parser,
    branch::alt,
    bytes::complete::{tag, take_until, take_while, take_while1},
    character::complete::{alphanumeric1, char, one_of, space0, space1},
    combinator::{map, recognize, value, all_consuming},
    multi::{many0, separated_list0},
    sequence::{delimited, preceded, pair},
    error::{context, ParseError as NomParseError},
};

use std::collections::HashSet;
use crate::cli::error::ParseError;
use crate::cli::types::{GlobalOption, TaskInvocation, TaskArgument, RawParsedCommand};

pub type ParseResult<'a, T> = IResult<&'a str, T, ParseError>;

// Basic building blocks
pub fn whitespace(input: &str) -> ParseResult<()> {
    value((), space0).parse(input)
}

pub fn whitespace1(input: &str) -> ParseResult<()> {
    value((), space1).parse(input)
}

pub fn identifier(input: &str) -> ParseResult<&str> {
    context(
        "identifier",
        recognize(pair(
            alt((alphanumeric1, tag("_"))),
            take_while(|c: char| c.is_alphanumeric() || c == '_' || c == '-')
        ))
    ).parse(input)
}

pub fn quoted_string(input: &str) -> ParseResult<String> {
    context(
        "quoted string",
        alt((
            delimited(
                char('"'),
                map(take_until("\""), |s: &str| s.to_string()),
                char('"')
            ),
            delimited(
                char('\''),
                map(take_until("'"), |s: &str| s.to_string()),
                char('\'')
            ),
        ))
    ).parse(input)
}

pub fn unquoted_value(input: &str) -> ParseResult<String> {
    context(
        "unquoted value",
        map(
            take_while1(|c: char| !c.is_whitespace()),
            |s: &str| s.to_string()
        )
    ).parse(input)
}

pub fn argument_value(input: &str) -> ParseResult<String> {
    context(
        "argument value",
        alt((quoted_string, unquoted_value))
    ).parse(input)
}

// Global option parsers
pub fn global_option_long_with_equals(input: &str) -> ParseResult<GlobalOption> {
    context(
        "global option with equals",
        map(
            (
                tag("--"),
                identifier,
                tag("="),
                argument_value,
            ),
            |(_, name, _, value)| GlobalOption {
                name: name.to_string(),
                value: Some(value),
            }
        )
    ).parse(input)
}

pub fn global_option_long_with_space(input: &str) -> ParseResult<GlobalOption> {
    context(
        "global option with space",
        map(
            (
                tag("--"),
                identifier,
                whitespace1,
                argument_value,
            ),
            |(_, name, _, value)| GlobalOption {
                name: name.to_string(),
                value: Some(value),
            }
        )
    ).parse(input)
}

pub fn global_option_short_with_space(input: &str) -> ParseResult<GlobalOption> {
    context(
        "short global option with space",
        map(
            (
                char('-'),
                one_of("oajHtvV"),  // Known short global options
                whitespace1,
                argument_value,
            ),
            |(_, short_char, _, value)| {
                let name = match short_char {
                    'o' => "ottofile",
                    'a' => "api",
                    'j' => "jobs",
                    'H' => "home",
                    't' => "tasks",
                    'v' => "verbosity",
                    _ => unreachable!(),
                };
                GlobalOption {
                    name: name.to_string(),
                    value: Some(value),
                }
            }
        )
    ).parse(input)
}

pub fn global_option_flag(input: &str) -> ParseResult<GlobalOption> {
    context(
        "global option flag",
        alt((
            map(tag("--help"), |_| GlobalOption {
                name: "help".to_string(),
                value: None,
            }),
            map(tag("-h"), |_| GlobalOption {
                name: "help".to_string(),
                value: None,
            }),
            map(tag("--version"), |_| GlobalOption {
                name: "version".to_string(),
                value: None,
            }),
            map(tag("-V"), |_| GlobalOption {
                name: "version".to_string(),
                value: None,
            }),
            map(tag("--verbose"), |_| GlobalOption {
                name: "verbose".to_string(),
                value: None,
            }),
        ))
    ).parse(input)
}

pub fn global_option(input: &str) -> ParseResult<GlobalOption> {
    context(
        "global option",
        alt((
            global_option_long_with_equals,
            global_option_flag,
            global_option_short_with_space,
            global_option_long_with_space,
        ))
    ).parse(input)
}

// Task argument parsers
pub fn task_argument_long_with_equals(input: &str) -> ParseResult<TaskArgument> {
    context(
        "task argument with equals",
        map(
            (
                tag("--"),
                identifier,
                tag("="),
                argument_value,
            ),
            |(_, name, _, value)| TaskArgument {
                name: name.to_string(),
                value: Some(value),
            }
        )
    ).parse(input)
}

pub fn task_argument_long_with_space(input: &str) -> ParseResult<TaskArgument> {
    context(
        "task argument with space",
        map(
            (
                tag("--"),
                identifier,
                whitespace1,
                argument_value,
            ),
            |(_, name, _, value)| TaskArgument {
                name: name.to_string(),
                value: Some(value),
            }
        )
    ).parse(input)
}

pub fn task_argument_short_with_space(input: &str) -> ParseResult<TaskArgument> {
    context(
        "short task argument with space",
        map(
            (
                char('-'),
                one_of("abcdefghijklmnopqrstuvwxyz"),  // Any lowercase letter
                whitespace1,
                argument_value,
            ),
            |(_, short_char, _, value)| TaskArgument {
                name: short_char.to_string(),
                value: Some(value),
            }
        )
    ).parse(input)
}

pub fn task_argument_flag(input: &str) -> ParseResult<TaskArgument> {
    context(
        "task argument flag",
        alt((
            map(preceded(tag("--"), identifier), |name| TaskArgument {
                name: name.to_string(),
                value: None,
            }),
            map(preceded(char('-'), one_of("abcdefghijklmnopqrstuvwxyz")), |short_char| TaskArgument {
                name: short_char.to_string(),
                value: None,
            }),
        ))
    ).parse(input)
}

pub fn task_argument(input: &str) -> ParseResult<TaskArgument> {
    context(
        "task argument",
        alt((
            task_argument_long_with_equals,
            task_argument_flag,  // Try flags before arguments with space
            task_argument_long_with_space,
            task_argument_short_with_space,
        ))
    ).parse(input)
}

/// Config-aware task argument parser that uses known task names to resolve ambiguity
pub fn task_argument_with_config(known_tasks: &HashSet<String>) -> impl Fn(&str) -> ParseResult<TaskArgument> + '_ {
    move |input| {
        // Try arguments with equals first (highest precedence)
        if let Ok((remaining, arg)) = task_argument_long_with_equals(input) {
            return Ok((remaining, arg));
        }

        // Try short arguments with space
        if let Ok((remaining, arg)) = task_argument_short_with_space(input) {
            return Ok((remaining, arg));
        }

        // For long arguments without equals, we need to disambiguate
        // --flag vs --flag value
        if let Ok((after_flag, flag_name)) = preceded(tag("--"), identifier).parse(input) {
            // Look ahead to see what follows the flag
            if let Ok((after_space, _)) = whitespace1.parse(after_flag) {
                // There's a space, check what the next token is
                if let Ok((_, next_token)) = identifier.parse(after_space) {
                    if known_tasks.contains(next_token) {
                        // Next token is a task name, so this is a flag
                        return Ok((after_flag, TaskArgument {
                            name: flag_name.to_string(),
                            value: None,
                        }));
                    }
                }

                // Check if the next token is another flag (starts with -)
                if after_space.starts_with('-') {
                    // Next token is a flag, so this is a boolean flag
                    return Ok((after_flag, TaskArgument {
                        name: flag_name.to_string(),
                        value: None,
                    }));
                }

                // Next token is not a task name or flag, try to parse as argument with value
                if let Ok((remaining, value)) = argument_value.parse(after_space) {
                    return Ok((remaining, TaskArgument {
                        name: flag_name.to_string(),
                        value: Some(value),
                    }));
                }
            }

            // No space after flag, treat as boolean flag
            return Ok((after_flag, TaskArgument {
                name: flag_name.to_string(),
                value: None,
            }));
        }

        // Try single character flags
        if let Ok((remaining, arg)) = task_argument_flag(input) {
            return Ok((remaining, arg));
        }

        // If nothing worked, return an error
        Err(nom::Err::Error(ParseError::from_error_kind(input, nom::error::ErrorKind::Alt)))
    }
}

pub fn task_name(input: &str) -> ParseResult<&str> {
    context("task name", identifier).parse(input)
}

pub fn task_invocation(input: &str) -> ParseResult<TaskInvocation> {
    context(
        "task invocation",
        map(
            pair(
                task_name,
                many0(preceded(whitespace1, task_argument))
            ),
            |(name, arguments)| TaskInvocation {
                name: name.to_string(),
                arguments,
            }
        )
    ).parse(input)
}

/// Config-aware task invocation parser
pub fn task_invocation_with_config(known_tasks: &HashSet<String>) -> impl Fn(&str) -> ParseResult<TaskInvocation> + '_ {
    move |input| {
        context(
            "config-aware task invocation",
            map(
                pair(
                    task_name,
                    many0(preceded(whitespace1, task_argument_with_config(known_tasks)))
                ),
                |(name, arguments)| TaskInvocation {
                    name: name.to_string(),
                    arguments,
                }
            )
        ).parse(input)
    }
}

pub fn command_line(input: &str) -> ParseResult<RawParsedCommand> {
    context(
        "command line",
        map(
            (
                many0(preceded(whitespace, global_option)),
                whitespace,
                separated_list0(whitespace1, task_invocation),
                whitespace,
            ),
            |(global_options, _, tasks, _)| RawParsedCommand {
                global_options,
                tasks,
            }
        )
    ).parse(input)
}

pub fn parse_command_line(input: &str) -> ParseResult<RawParsedCommand> {
    all_consuming(command_line).parse(input)
}

/// Config-aware command line parser - for complete parsing with config
pub fn parse_command_line_with_config<'a>(input: &'a str, known_tasks: &'a HashSet<String>) -> ParseResult<'a, RawParsedCommand> {
    context(
        "config-aware command line",
        all_consuming(
            map(
                (
                    many0(preceded(whitespace, global_option)),
                    whitespace,
                    separated_list0(whitespace1, task_invocation_with_config(known_tasks)),
                    whitespace,
                ),
                |(global_options, _, tasks, _)| RawParsedCommand {
                    global_options,
                    tasks,
                }
            )
        )
    ).parse(input)
}

/// Parse task invocations only (no global options) - for Pass 2
pub fn parse_task_invocations_only(input: &str) -> ParseResult<Vec<TaskInvocation>> {
    context(
        "task invocations only",
        all_consuming(
            map(
                (
                    whitespace,
                    separated_list0(whitespace1, task_invocation),
                    whitespace,
                ),
                |(_, tasks, _)| tasks
            )
        )
    ).parse(input)
}

/// Config-aware task invocations parser - for Pass 2 with config
pub fn parse_task_invocations_with_config<'a>(input: &'a str, known_tasks: &'a HashSet<String>) -> ParseResult<'a, Vec<TaskInvocation>> {
    context(
        "config-aware task invocations",
        all_consuming(
            map(
                (
                    whitespace,
                    separated_list0(whitespace1, task_invocation_with_config(known_tasks)),
                    whitespace,
                ),
                |(_, tasks, _)| tasks
            )
        )
    ).parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_argument_with_config_disambiguation() {
        let mut known_tasks = HashSet::new();
        known_tasks.insert("world".to_string());
        known_tasks.insert("test".to_string());

        // Test case: --flag taskname (should be boolean flag)
        let result = task_argument_with_config(&known_tasks)("--verbose world");
        assert!(result.is_ok());
        let (remaining, arg) = result.unwrap();
        assert_eq!(remaining, " world");
        assert_eq!(arg.name, "verbose");
        assert_eq!(arg.value, None); // Boolean flag

        // Test case: --flag value (should take value)
        let result = task_argument_with_config(&known_tasks)("--greeting hello");
        assert!(result.is_ok());
        let (remaining, arg) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(arg.name, "greeting");
        assert_eq!(arg.value, Some("hello".to_string()));

        // Test case: --flag=value (should always take value)
        let result = task_argument_with_config(&known_tasks)("--greeting=hello");
        assert!(result.is_ok());
        let (remaining, arg) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(arg.name, "greeting");
        assert_eq!(arg.value, Some("hello".to_string()));
    }

    #[test]
    fn test_task_invocation_with_config() {
        let mut known_tasks = HashSet::new();
        known_tasks.insert("world".to_string());
        known_tasks.insert("test".to_string());

        // Test parsing task with config-aware arguments
        let result = task_invocation_with_config(&known_tasks)("hello --verbose --greeting value");
        assert!(result.is_ok());
        let (remaining, invocation) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(invocation.name, "hello");
        assert_eq!(invocation.arguments.len(), 2);

        // First argument should be boolean flag
        assert_eq!(invocation.arguments[0].name, "verbose");
        assert_eq!(invocation.arguments[0].value, None);

        // Second argument should take value
        assert_eq!(invocation.arguments[1].name, "greeting");
        assert_eq!(invocation.arguments[1].value, Some("value".to_string()));
    }

    #[test]
    fn test_parse_task_invocations_with_config() {
        let mut known_tasks = HashSet::new();
        known_tasks.insert("world".to_string());
        known_tasks.insert("test".to_string());

        let result = parse_task_invocations_with_config("hello --verbose world --flag", &known_tasks);
        assert!(result.is_ok());
        let (remaining, invocations) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(invocations.len(), 2);

        // First task: hello --verbose (verbose is boolean because world is next)
        assert_eq!(invocations[0].name, "hello");
        assert_eq!(invocations[0].arguments.len(), 1);
        assert_eq!(invocations[0].arguments[0].name, "verbose");
        assert_eq!(invocations[0].arguments[0].value, None);

        // Second task: world --flag (flag is boolean)
        assert_eq!(invocations[1].name, "world");
        assert_eq!(invocations[1].arguments.len(), 1);
        assert_eq!(invocations[1].arguments[0].name, "flag");
        assert_eq!(invocations[1].arguments[0].value, None);
    }

    #[test]
    fn test_whitespace() {
        let result = whitespace("   hello");
        assert!(result.is_ok());
        let (remaining, _) = result.unwrap();
        assert_eq!(remaining, "hello");
    }

    #[test]
    fn test_identifier() {
        let result = identifier("hello-world_123");
        assert!(result.is_ok());
        let (remaining, id) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(id, "hello-world_123");
    }

    #[test]
    fn test_quoted_string() {
        let result = quoted_string("\"hello world\"");
        assert!(result.is_ok());
        let (remaining, s) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(s, "hello world");
    }

    #[test]
    fn test_task_argument_long_with_equals() {
        let result = task_argument_long_with_equals("--greeting=hello");
        assert!(result.is_ok());
        let (remaining, arg) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(arg.name, "greeting");
        assert_eq!(arg.value, Some("hello".to_string()));
    }

    #[test]
    fn test_task_argument_flag() {
        let result = task_argument_flag("--verbose");
        assert!(result.is_ok());
        let (remaining, arg) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(arg.name, "verbose");
        assert_eq!(arg.value, None);
    }

    #[test]
    fn test_task_invocation() {
        let result = task_invocation("hello --greeting=world --verbose");
        assert!(result.is_ok());
        let (remaining, invocation) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(invocation.name, "hello");
        assert_eq!(invocation.arguments.len(), 2);

        assert_eq!(invocation.arguments[0].name, "greeting");
        assert_eq!(invocation.arguments[0].value, Some("world".to_string()));

        assert_eq!(invocation.arguments[1].name, "verbose");
        assert_eq!(invocation.arguments[1].value, None);
    }

    #[test]
    fn test_parse_task_invocations_only() {
        let result = parse_task_invocations_only("hello --greeting=world test --flag");
        assert!(result.is_ok());
        let (remaining, invocations) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(invocations.len(), 2);

        assert_eq!(invocations[0].name, "hello");
        assert_eq!(invocations[0].arguments.len(), 1);
        assert_eq!(invocations[0].arguments[0].name, "greeting");
        assert_eq!(invocations[0].arguments[0].value, Some("world".to_string()));

        assert_eq!(invocations[1].name, "test");
        assert_eq!(invocations[1].arguments.len(), 1);
        assert_eq!(invocations[1].arguments[0].name, "flag");
        assert_eq!(invocations[1].arguments[0].value, None);
    }
}
