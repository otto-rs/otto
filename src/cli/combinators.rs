use nom::{
    IResult, Parser,
    branch::alt,
    bytes::complete::{tag, take_until, take_while, take_while1},
    character::complete::{alphanumeric1, char, one_of, space0, space1},
    combinator::{map, recognize, value, all_consuming},
    multi::{many0, separated_list0},
    sequence::{delimited, preceded, pair},
    error::context,
};

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
                one_of("oajHtvT"),  // Known short options
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

                    _ => "unknown",
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
            task_argument_long_with_space,
            task_argument_short_with_space,
            task_argument_flag,
        ))
    ).parse(input)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identifier() {
        assert_eq!(identifier("hello world"), Ok((" world", "hello")));
        assert_eq!(identifier("task-name rest"), Ok((" rest", "task-name")));
        assert_eq!(identifier("task_name rest"), Ok((" rest", "task_name")));
    }

    #[test]
    fn test_global_option_with_equals() {
        let result = global_option_long_with_equals("--ottofile=path");
        assert!(result.is_ok());
        let (_, option) = result.unwrap();
        assert_eq!(option.name, "ottofile");
        assert_eq!(option.value, Some("path".to_string()));
    }

    #[test]
    fn test_global_option_flag() {
        let result = global_option_flag("--help");
        assert!(result.is_ok());
        let (_, option) = result.unwrap();
        assert_eq!(option.name, "help");
        assert_eq!(option.value, None);
    }

    #[test]
    fn test_task_argument_with_equals() {
        let result = task_argument_long_with_equals("--greeting=hello");
        assert!(result.is_ok());
        let (_, arg) = result.unwrap();
        assert_eq!(arg.name, "greeting");
        assert_eq!(arg.value, Some("hello".to_string()));
    }

    #[test]
    fn test_task_invocation() {
        let result = task_invocation("hello --greeting=world");
        assert!(result.is_ok());
        let (_, task) = result.unwrap();
        assert_eq!(task.name, "hello");
        assert_eq!(task.arguments.len(), 1);
        assert_eq!(task.arguments[0].name, "greeting");
        assert_eq!(task.arguments[0].value, Some("world".to_string()));
    }

    #[test]
    fn test_simple_command() {
        let result = parse_command_line("hello");
        assert!(result.is_ok());
        let (_, cmd) = result.unwrap();
        assert_eq!(cmd.tasks.len(), 1);
        assert_eq!(cmd.tasks[0].name, "hello");
    }

    #[test]
    fn test_complex_command() {
        let result = parse_command_line("--ottofile=config.yml hello --greeting=world test --verbose");
        assert!(result.is_ok());
        let (_, cmd) = result.unwrap();
        assert_eq!(cmd.global_options.len(), 1);
        assert_eq!(cmd.global_options[0].name, "ottofile");
        assert_eq!(cmd.tasks.len(), 2);
        assert_eq!(cmd.tasks[0].name, "hello");
        assert_eq!(cmd.tasks[1].name, "test");
    }
}
