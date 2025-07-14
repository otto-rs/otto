use nom::{
    IResult, Parser,
    branch::alt,
    bytes::complete::{tag, take_while},
    character::complete::{char, one_of, space1},
    combinator::{recognize, all_consuming, map},
    error::context,
};

use crate::cli::types::GlobalOption;
use crate::cli::error::ParseError;
use crate::cli::combinators::{whitespace, argument_value};

pub type ParseResult<'a, T> = IResult<&'a str, T, ParseError>;

/// Parse only global options, return remaining input for Pass 2
/// This is the core of Pass 1 - it consumes known global options and leaves everything else
pub fn parse_global_options_only(input: &str) -> Result<(Vec<GlobalOption>, String), ParseError> {
    let input = input.trim();

    // Handle empty input
    if input.is_empty() {
        return Ok((vec![], String::new()));
    }

    // Parse global options and capture remaining
    match parse_globals_and_remaining(input) {
        Ok((remaining, (global_opts, remaining_str))) => {
            if !remaining.trim().is_empty() {
                return Err(ParseError::UnconsumedInput {
                    remaining: remaining.to_string(),
                });
            }
            Ok((global_opts, remaining_str))
        }
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => Err(e),
        Err(nom::Err::Incomplete(_)) => Err(ParseError::IncompleteInput),
    }
}

/// Internal parser that extracts global options and remaining args
fn parse_globals_and_remaining(input: &str) -> ParseResult<(Vec<GlobalOption>, String)> {
    context(
        "global options and remaining args",
        all_consuming(extract_global_options_and_remaining)
    ).parse(input)
}

/// Extract global options using a hybrid approach
/// This parses global options that can appear anywhere in the input
/// NOTE: This is a compromise - it uses nom combinators for individual parsing
/// but falls back to a controlled loop for the overall structure
fn extract_global_options_and_remaining(input: &str) -> ParseResult<(Vec<GlobalOption>, String)> {
    let mut global_options = Vec::new();
    let mut remaining_parts = Vec::new();
    let mut current_input = input;

    while !current_input.trim().is_empty() {
        // Skip whitespace using nom
        let (after_space, _) = whitespace(current_input)?;
        current_input = after_space;

        if current_input.is_empty() {
            break;
        }

        // Try to parse a global option using nom
        match known_global_option(current_input) {
            Ok((remaining, option)) => {
                global_options.push(option);
                current_input = remaining;
            }
            Err(_) => {
                // Not a global option, parse next token using nom
                let (remaining, token) = next_token(current_input)?;
                remaining_parts.push(token);
                current_input = remaining;
            }
        }
    }

    let remaining_str = remaining_parts.join(" ");
    Ok(("", (global_options, remaining_str)))
}

/// Parse the next token (word or quoted string) for remaining args
fn next_token(input: &str) -> ParseResult<&str> {
    context(
        "next token",
        alt((
            // Quoted strings
            recognize((
                tag("\""),
                take_while(|c| c != '"'),
                tag("\""),
            )),
            recognize((
                tag("'"),
                take_while(|c| c != '\''),
                tag("'"),
            )),
            // Regular tokens (stop at whitespace)
            take_while(|c: char| !c.is_whitespace()),
        ))
    ).parse(input)
}

/// Parse only known global options (restrictive parser for Pass 1)
fn known_global_option(input: &str) -> ParseResult<GlobalOption> {
    context(
        "known global option",
        alt((
            // Long options with equals
            known_global_option_long_with_equals,
            // Flags (no value)
            known_global_option_flag,
            // Short options with space
            known_global_option_short_with_space,
            // Long options with space
            known_global_option_long_with_space,
        ))
    ).parse(input)
}

/// Parse known global options with equals (--ottofile=value)
fn known_global_option_long_with_equals(input: &str) -> ParseResult<GlobalOption> {
    context(
        "known global option with equals",
        alt((
            map(
                (tag("--ottofile="), argument_value),
                |(_, value)| GlobalOption {
                    name: "ottofile".to_string(),
                    value: Some(value),
                }
            ),
            map(
                (tag("--api="), argument_value),
                |(_, value)| GlobalOption {
                    name: "api".to_string(),
                    value: Some(value),
                }
            ),
            map(
                (tag("--jobs="), argument_value),
                |(_, value)| GlobalOption {
                    name: "jobs".to_string(),
                    value: Some(value),
                }
            ),
            map(
                (tag("--home="), argument_value),
                |(_, value)| GlobalOption {
                    name: "home".to_string(),
                    value: Some(value),
                }
            ),
            map(
                (tag("--tasks="), argument_value),
                |(_, value)| GlobalOption {
                    name: "tasks".to_string(),
                    value: Some(value),
                }
            ),
            map(
                (tag("--verbosity="), argument_value),
                |(_, value)| GlobalOption {
                    name: "verbosity".to_string(),
                    value: Some(value),
                }
            ),
        ))
    ).parse(input)
}

/// Parse known global option flags (--help, --version, --verbose)
fn known_global_option_flag(input: &str) -> ParseResult<GlobalOption> {
    context(
        "known global option flag",
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

/// Parse known short options with space (-o value)
fn known_global_option_short_with_space(input: &str) -> ParseResult<GlobalOption> {
    context(
        "known short global option with space",
        map(
            (
                char('-'),
                one_of("oajHtv"),  // Known short options
                                 space1,
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

/// Parse known long options with space (--ottofile value)
fn known_global_option_long_with_space(input: &str) -> ParseResult<GlobalOption> {
    context(
        "known global option with space",
        alt((
            map(
                (tag("--ottofile"), space1, argument_value),
                |(_, _, value)| GlobalOption {
                    name: "ottofile".to_string(),
                    value: Some(value),
                }
            ),
            map(
                (tag("--api"), space1, argument_value),
                |(_, _, value)| GlobalOption {
                    name: "api".to_string(),
                    value: Some(value),
                }
            ),
            map(
                (tag("--jobs"), space1, argument_value),
                |(_, _, value)| GlobalOption {
                    name: "jobs".to_string(),
                    value: Some(value),
                }
            ),
            map(
                (tag("--home"), space1, argument_value),
                |(_, _, value)| GlobalOption {
                    name: "home".to_string(),
                    value: Some(value),
                }
            ),
            map(
                (tag("--tasks"), space1, argument_value),
                |(_, _, value)| GlobalOption {
                    name: "tasks".to_string(),
                    value: Some(value),
                }
            ),
            map(
                (tag("--verbosity"), space1, argument_value),
                |(_, _, value)| GlobalOption {
                    name: "verbosity".to_string(),
                    value: Some(value),
                }
            ),
        ))
    ).parse(input)
}

/// Parse the next token (word or quoted string) for remaining args


#[cfg(test)]
mod tests {
    use super::*;



    #[test]
    fn test_empty_input() {
        let result = parse_global_options_only("");
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();
        assert!(global_opts.is_empty());
        assert_eq!(remaining, "");
    }

    #[test]
    fn test_only_global_options() {
        let result = parse_global_options_only("--ottofile config.yml --verbose");
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();
        assert_eq!(global_opts.len(), 2);
        assert_eq!(global_opts[0].name, "ottofile");
        assert_eq!(global_opts[0].value, Some("config.yml".to_string()));
        assert_eq!(global_opts[1].name, "verbose");
        assert_eq!(global_opts[1].value, None);
        assert_eq!(remaining, "");
    }

    #[test]
    fn test_only_tasks() {
        let result = parse_global_options_only("hello --greeting world test --flag");
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();

        assert!(global_opts.is_empty());
        assert_eq!(remaining, "hello --greeting world test --flag");
    }

    #[test]
    fn test_mixed_global_and_task_args() {
        let result = parse_global_options_only("--ottofile config.yml hello --greeting world --verbose test --flag");
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();
        assert_eq!(global_opts.len(), 2);
        assert_eq!(global_opts[0].name, "ottofile");
        assert_eq!(global_opts[0].value, Some("config.yml".to_string()));
        assert_eq!(global_opts[1].name, "verbose");
        assert_eq!(global_opts[1].value, None);
        assert_eq!(remaining, "hello --greeting world test --flag");
    }

    #[test]
    fn test_help_flag() {
        let result = parse_global_options_only("--help");
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();
        assert_eq!(global_opts.len(), 1);
        assert_eq!(global_opts[0].name, "help");
        assert_eq!(global_opts[0].value, None);
        assert_eq!(remaining, "");
    }

    #[test]
    fn test_version_flag() {
        let result = parse_global_options_only("--version");
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();
        assert_eq!(global_opts.len(), 1);
        assert_eq!(global_opts[0].name, "version");
        assert_eq!(remaining, "");
    }

    #[test]
    fn test_short_flags() {
        let result = parse_global_options_only("-o config.yml -v hello world");
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();
        assert_eq!(global_opts.len(), 2);
        assert_eq!(global_opts[0].name, "ottofile");
        assert_eq!(global_opts[0].value, Some("config.yml".to_string()));
        assert_eq!(global_opts[1].name, "verbosity");
        assert_eq!(global_opts[1].value, Some("hello".to_string()));
        assert_eq!(remaining, "world");
    }

    #[test]
    fn test_global_options_with_equals() {
        let result = parse_global_options_only("--ottofile=config.yml --jobs=4 hello");
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();
        assert_eq!(global_opts.len(), 2);
        assert_eq!(global_opts[0].name, "ottofile");
        assert_eq!(global_opts[0].value, Some("config.yml".to_string()));
        assert_eq!(global_opts[1].name, "jobs");
        assert_eq!(global_opts[1].value, Some("4".to_string()));
        assert_eq!(remaining, "hello");
    }

    #[test]
    fn test_quoted_values() {
        let result = parse_global_options_only("--ottofile \"my config.yml\" hello --greeting \"hello world\"");
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();
        assert_eq!(global_opts.len(), 1);
        assert_eq!(global_opts[0].name, "ottofile");
        assert_eq!(global_opts[0].value, Some("my config.yml".to_string()));
        assert_eq!(remaining, "hello --greeting \"hello world\"");
    }

    #[test]
    fn test_complex_mixed_case() {
        let input = "--ottofile custom.yml --verbose hello --greeting world --jobs 2 test --flag -h";
        let result = parse_global_options_only(input);
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();

        // Should extract: --ottofile, --verbose, --jobs, -h
        assert_eq!(global_opts.len(), 4);

        // Check ottofile
        assert_eq!(global_opts[0].name, "ottofile");
        assert_eq!(global_opts[0].value, Some("custom.yml".to_string()));

        // Check verbose
        assert_eq!(global_opts[1].name, "verbose");
        assert_eq!(global_opts[1].value, None);

        // Check jobs
        assert_eq!(global_opts[2].name, "jobs");
        assert_eq!(global_opts[2].value, Some("2".to_string()));

        // Check help
        assert_eq!(global_opts[3].name, "help");
        assert_eq!(global_opts[3].value, None);

        // Remaining should be task-related args
        assert_eq!(remaining, "hello --greeting world test --flag");
    }

    #[test]
    fn test_whitespace_handling() {
        let result = parse_global_options_only("  --ottofile   config.yml   hello   --greeting   world  ");
        assert!(result.is_ok());
        let (global_opts, remaining) = result.unwrap();
        assert_eq!(global_opts.len(), 1);
        assert_eq!(global_opts[0].name, "ottofile");
        assert_eq!(global_opts[0].value, Some("config.yml".to_string()));
        assert_eq!(remaining, "hello --greeting world");
    }
}
