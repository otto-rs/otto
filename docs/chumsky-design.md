# Chumsky Parser Design for Otto CLI

## Table of Contents

1. [Overview](#overview)
2. [Why Chumsky for Otto](#why-chumsky-for-otto)
3. [Architecture](#architecture)
4. [Implementation Plan](#implementation-plan)
5. [Core Parser Components](#core-parser-components)
6. [Context-Aware Parsing](#context-aware-parsing)
7. [Error Handling](#error-handling)
8. [Migration Strategy](#migration-strategy)
9. [Performance Considerations](#performance-considerations)
10. [Testing Strategy](#testing-strategy)
11. [Examples](#examples)
12. [Dependencies](#dependencies)

## Overview

This document outlines the design for migrating Otto's CLI parser from the current hybrid nom/bespoke approach to Chumsky, a modern parser combinator library designed for context-sensitive grammars.

### Current Problems

Otto's CLI parsing has several challenges that the current nom-based approach struggles with:

1. **Context-sensitive disambiguation**: `--flag taskname` vs `--flag value` depends on runtime config
2. **Two-pass complexity**: Global options extraction → config loading → task parsing creates architectural friction
3. **Hybrid implementation**: Mix of nom combinators and manual recursive descent is hard to maintain
4. **Limited error recovery**: Poor error messages for domain-specific parsing failures

### Chumsky Solution

Chumsky addresses these issues through:

- **Stateful parsing**: Parsers can carry configuration context
- **Superior error recovery**: Built-in error reporting with custom error types
- **Unified architecture**: Single parser library handles all parsing needs
- **Composable combinators**: Clean separation of concerns while maintaining flexibility

## Why Chumsky for Otto

### Context-Sensitive Grammar Support

Otto's grammar is inherently context-sensitive:

```bash
# These parse differently based on config
otto --verbose hello    # --verbose flag + hello task (if hello is known task)
otto --verbose hello    # --verbose="hello" (if hello is not a known task)
```

Chumsky's `map_with_state` enables this naturally:

```rust
let task_arg = just("--")
    .ignore_then(ident())
    .then(value().or_not())
    .map_with_state(|(flag, value), config: &Config| {
        match value {
            Some(v) if config.tasks.contains_key(&v) => TaskArg::flag(flag),
            Some(v) => TaskArg::with_value(flag, v),
            None => TaskArg::flag(flag),
        }
    });
```

### Two-Pass Architecture Integration

Chumsky's design naturally supports Otto's two-pass approach:

```rust
// Pass 1: Extract globals (stateless)
let global_parser = global_options().then_ignore(end());

// Pass 2: Parse tasks (stateful with config)
let task_parser = task_invocations()
    .configure(config)  // Inject config as state
    .then_ignore(end());
```

### Error Quality

Otto needs domain-specific error messages:

```rust
#[derive(Debug, Clone)]
enum OttoParseError {
    UnknownTask { name: String, suggestions: Vec<String> },
    UnknownTaskArgument { task: String, arg: String },
    InvalidArgumentValue { arg: String, value: String, expected: String },
    ConfigRequired,
}

impl chumsky::error::Error<char> for OttoParseError {
    // Custom error formatting
}
```

## Architecture

### High-Level Design

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   Raw Input     │───▶│   Chumsky        │───▶│   Parsed        │
│   Command Line  │    │   Parser         │    │   Command       │
└─────────────────┘    └──────────────────┘    └─────────────────┘
                              │
                              ▼
                       ┌──────────────────┐
                       │   Config State   │
                       │   (known tasks,  │
                       │    param specs)  │
                       └──────────────────┘
```

### Module Structure

```
src/cli/
├── chumsky/
│   ├── mod.rs              # Public API
│   ├── lexer.rs            # Token definitions
│   ├── primitives.rs       # Basic combinators
│   ├── global_options.rs   # Global option parsers
│   ├── tasks.rs            # Task parsing with config
│   ├── commands.rs         # Built-in commands (graph, help)
│   ├── errors.rs           # Custom error types
│   └── state.rs            # Parser state management
├── types.rs                # AST types (shared)
└── validation.rs           # Post-parse validation
```

## Implementation Plan

### Phase 1: Core Infrastructure (Week 1)

1. **Add Chumsky dependency**
   ```toml
   [dependencies]
   chumsky = "0.9"
   ```

2. **Define token types and basic combinators**
   ```rust
   // lexer.rs
   #[derive(Debug, Clone, PartialEq)]
   pub enum Token {
       Ident(String),
       String(String),
       DoubleDash,
       SingleDash,
       Equals,
   }
   ```

3. **Create parser state structure**
   ```rust
   // state.rs
   #[derive(Debug, Clone)]
   pub struct ParserState {
       pub config: Option<ConfigSpec>,
       pub known_tasks: HashSet<String>,
       pub global_options: HashMap<String, ParamSpec>,
   }
   ```

### Phase 2: Global Options Parser (Week 1)

1. **Implement global option combinators**
2. **Add comprehensive tests**
3. **Ensure backward compatibility**

### Phase 3: Task Parsing with Context (Week 2)

1. **Implement context-aware task argument parsing**
2. **Add config state injection**
3. **Handle disambiguation logic**

### Phase 4: Integration & Testing (Week 2)

1. **Replace nom parser in main.rs**
2. **Run full test suite**
3. **Performance benchmarking**

### Phase 5: Error Handling & Polish (Week 3)

1. **Implement custom error types**
2. **Add suggestion system**
3. **Improve error messages**

## Core Parser Components

### Lexer (Token-Based Approach)

```rust
// lexer.rs
use chumsky::prelude::*;

pub fn lexer() -> impl Parser<char, Vec<Token>, Error = Simple<char>> {
    let ident = text::ident().map(Token::Ident);
    
    let string = just('"')
        .ignore_then(filter(|c| *c != '"').repeated())
        .then_ignore(just('"'))
        .collect::<String>()
        .map(Token::String)
        .or(
            just('\'')
                .ignore_then(filter(|c| *c != '\'').repeated())
                .then_ignore(just('\''))
                .collect::<String>()
                .map(Token::String)
        );
    
    let double_dash = just("--").to(Token::DoubleDash);
    let single_dash = just("-").to(Token::SingleDash);
    let equals = just("=").to(Token::Equals);
    
    choice((string, double_dash, single_dash, equals, ident))
        .padded()
        .repeated()
}
```

### Primitive Combinators

```rust
// primitives.rs
use chumsky::prelude::*;
use crate::cli::types::*;

pub fn identifier() -> impl Parser<Token, String, Error = Simple<Token>> {
    select! {
        Token::Ident(name) => name,
    }
}

pub fn string_value() -> impl Parser<Token, String, Error = Simple<Token>> {
    select! {
        Token::String(s) => s,
        Token::Ident(s) => s,  // Unquoted strings
    }
}

pub fn long_flag() -> impl Parser<Token, String, Error = Simple<Token>> {
    just(Token::DoubleDash)
        .ignore_then(identifier())
}

pub fn short_flag() -> impl Parser<Token, String, Error = Simple<Token>> {
    just(Token::SingleDash)
        .ignore_then(select! {
            Token::Ident(s) if s.len() == 1 => s,
        })
}
```

### Global Options Parser

```rust
// global_options.rs
use chumsky::prelude::*;
use crate::cli::types::*;

pub fn global_option() -> impl Parser<Token, GlobalOption, Error = Simple<Token>> {
    choice((
        global_option_with_equals(),
        global_option_flag(),
        global_option_with_space(),
    ))
}

fn global_option_with_equals() -> impl Parser<Token, GlobalOption, Error = Simple<Token>> {
    long_flag()
        .then_ignore(just(Token::Equals))
        .then(string_value())
        .try_map(|(name, value), span| {
            if is_known_global_option(&name) {
                Ok(GlobalOption { name, value: Some(value) })
            } else {
                Err(Simple::custom(span, format!("Unknown global option: {}", name)))
            }
        })
}

fn global_option_flag() -> impl Parser<Token, GlobalOption, Error = Simple<Token>> {
    long_flag()
        .try_map(|name, span| {
            if is_known_global_flag(&name) {
                Ok(GlobalOption { name, value: None })
            } else {
                Err(Simple::custom(span, format!("Unknown global flag: {}", name)))
            }
        })
}

fn is_known_global_option(name: &str) -> bool {
    matches!(name, "ottofile" | "api" | "jobs" | "home" | "tasks" | "verbosity")
}

fn is_known_global_flag(name: &str) -> bool {
    matches!(name, "help" | "version" | "verbose")
}
```

### Context-Aware Task Parsing

```rust
// tasks.rs
use chumsky::prelude::*;
use crate::cli::types::*;
use crate::cli::state::ParserState;

pub fn task_invocation() -> impl Parser<Token, TaskInvocation, Error = OttoParseError> + Clone {
    identifier()
        .then(task_argument().repeated())
        .map(|(name, arguments)| TaskInvocation { name, arguments })
}

pub fn task_argument() -> impl Parser<Token, TaskArgument, Error = OttoParseError> + Clone {
    choice((
        task_argument_with_equals(),
        task_argument_with_space_context_aware(),
        task_argument_flag(),
    ))
}

fn task_argument_with_space_context_aware() -> impl Parser<Token, TaskArgument, Error = OttoParseError> + Clone {
    long_flag()
        .then(string_value().or_not())
        .map_with_state(|(flag, maybe_value), state: &ParserState| {
            match maybe_value {
                Some(value) => {
                    // Context-sensitive disambiguation
                    if state.known_tasks.contains(&value) {
                        // Next token is a known task, treat as flag
                        TaskArgument { name: flag, value: None }
                    } else {
                        // Next token is not a task, treat as argument value
                        TaskArgument { name: flag, value: Some(value) }
                    }
                }
                None => TaskArgument { name: flag, value: None }
            }
        })
}

pub fn task_invocations() -> impl Parser<Token, Vec<TaskInvocation>, Error = OttoParseError> + Clone {
    task_invocation()
        .separated_by(just(Token::Whitespace).ignored())
        .at_least(1)
        .collect()
}
```

## Context-Aware Parsing

### State Management

```rust
// state.rs
use std::collections::{HashMap, HashSet};
use crate::cfg::config::ConfigSpec;

#[derive(Debug, Clone)]
pub struct ParserState {
    pub config: Option<ConfigSpec>,
    pub known_tasks: HashSet<String>,
    pub global_param_specs: HashMap<String, GlobalParamSpec>,
}

impl ParserState {
    pub fn new(config: Option<ConfigSpec>) -> Self {
        let known_tasks = config
            .as_ref()
            .map(|c| c.tasks.keys().cloned().collect())
            .unwrap_or_default();
        
        Self {
            config,
            known_tasks,
            global_param_specs: Self::build_global_specs(),
        }
    }
    
    pub fn is_known_task(&self, name: &str) -> bool {
        self.known_tasks.contains(name)
    }
    
    pub fn get_task_param_spec(&self, task: &str, param: &str) -> Option<&ParamSpec> {
        self.config
            .as_ref()?
            .tasks
            .get(task)?
            .params
            .get(param)
    }
    
    fn build_global_specs() -> HashMap<String, GlobalParamSpec> {
        // Define global option specifications
        let mut specs = HashMap::new();
        specs.insert("ottofile".to_string(), GlobalParamSpec::required_string());
        specs.insert("jobs".to_string(), GlobalParamSpec::required_int());
        specs.insert("help".to_string(), GlobalParamSpec::flag());
        // ... etc
        specs
    }
}
```

### Disambiguation Logic

```rust
// Core disambiguation for --flag value vs --flag (boolean)
fn disambiguate_flag_argument(
    flag: String,
    next_token: Option<String>,
    state: &ParserState,
) -> TaskArgument {
    match next_token {
        Some(token) => {
            // Check if next token is a known task name
            if state.is_known_task(&token) {
                // --flag taskname → flag is boolean, taskname is separate
                TaskArgument { name: flag, value: None }
            } else if token.starts_with('-') {
                // --flag --otherflag → flag is boolean
                TaskArgument { name: flag, value: None }
            } else {
                // --flag value → flag takes value
                TaskArgument { name: flag, value: Some(token) }
            }
        }
        None => {
            // --flag (end of input) → boolean flag
            TaskArgument { name: flag, value: None }
        }
    }
}
```

## Error Handling

### Custom Error Types

```rust
// errors.rs
use chumsky::error::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum OttoParseError {
    // Token-level errors
    UnexpectedToken { expected: Vec<String>, found: String },
    UnexpectedEndOfInput { expected: Vec<String> },
    
    // Semantic errors
    UnknownTask { name: String, suggestions: Vec<String> },
    UnknownGlobalOption { name: String },
    UnknownTaskArgument { task: String, arg: String },
    
    // Validation errors
    InvalidArgumentValue { arg: String, value: String, expected: String },
    MissingRequiredArgument { task: String, arg: String },
    ConfigRequired,
    
    // Custom errors
    Custom(String),
}

impl Error<Token> for OttoParseError {
    type Span = SimpleSpan;
    
    fn expected_input_found<Iter: IntoIterator<Item = Option<Token>>>(
        span: Self::Span,
        expected: Iter,
        found: Option<Token>,
    ) -> Self {
        let expected: Vec<String> = expected
            .into_iter()
            .map(|t| format!("{:?}", t))
            .collect();
        
        let found = found
            .map(|t| format!("{:?}", t))
            .unwrap_or_else(|| "end of input".to_string());
        
        Self::UnexpectedToken { expected, found }
    }
}
```

### Error Recovery and Suggestions

```rust
// Error recovery for unknown tasks
fn parse_task_name_with_suggestions() -> impl Parser<Token, String, Error = OttoParseError> + Clone {
    identifier()
        .validate(|name, span, state: &ParserState| {
            if state.is_known_task(&name) {
                Ok(name)
            } else {
                let suggestions = suggest_similar_tasks(&name, &state.known_tasks);
                Err(OttoParseError::UnknownTask { name, suggestions })
            }
        })
}

fn suggest_similar_tasks(invalid: &str, valid_tasks: &HashSet<String>) -> Vec<String> {
    valid_tasks
        .iter()
        .filter_map(|task| {
            let distance = levenshtein::levenshtein(invalid, task);
            if distance <= 3 {
                Some((task.clone(), distance))
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .sorted_by_key(|(_, distance)| *distance)
        .take(3)
        .map(|(task, _)| task)
        .collect()
}
```

## Migration Strategy

### Phase 1: Parallel Implementation

1. **Keep existing nom parser** as fallback
2. **Add Chumsky parser** alongside
3. **Feature flag** to switch between parsers
4. **Comprehensive testing** of both parsers

```rust
// main.rs
#[cfg(feature = "chumsky-parser")]
use crate::cli::chumsky::ChumskyParser;

#[cfg(not(feature = "chumsky-parser"))]
use crate::cli::NomParser as Parser;

#[cfg(feature = "chumsky-parser")]
use crate::cli::chumsky::ChumskyParser as Parser;
```

### Phase 2: Testing & Validation

1. **Run both parsers** on same input
2. **Compare outputs** for consistency
3. **Performance benchmarking**
4. **Edge case testing**

### Phase 3: Gradual Rollout

1. **Default to Chumsky** with nom fallback
2. **Monitor for regressions**
3. **Remove nom parser** after confidence period

### Backward Compatibility

```rust
// Ensure identical public API
pub trait CliParser {
    fn parse(&mut self, input: &str) -> Result<ParsedCommand, ParseError>;
    fn parse_global_options_only(&self, input: &str) -> Result<(Vec<GlobalOption>, String), ParseError>;
    fn parse_tasks_only(&self, input: &str) -> Result<Vec<ParsedTask>, ParseError>;
}

impl CliParser for ChumskyParser {
    // Implementation maintains exact same behavior
}
```

## Performance Considerations

### Benchmarking Strategy

```rust
// benches/parser_comparison.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_parsers(c: &mut Criterion) {
    let inputs = vec![
        "otto --help",
        "otto --ottofile=custom.yml build --verbose",
        "otto test --coverage build --release deploy --env production",
        // ... more test cases
    ];
    
    let mut group = c.benchmark_group("parser_comparison");
    
    for input in inputs {
        group.bench_with_input(BenchmarkId::new("nom", input), &input, |b, input| {
            b.iter(|| nom_parser.parse(black_box(input)))
        });
        
        group.bench_with_input(BenchmarkId::new("chumsky", input), &input, |b, input| {
            b.iter(|| chumsky_parser.parse(black_box(input)))
        });
    }
    
    group.finish();
}
```

### Expected Performance

- **Chumsky overhead**: ~10-20% slower than nom for simple cases
- **Complex disambiguation**: Chumsky likely faster due to better architecture
- **Error cases**: Chumsky significantly faster due to better error recovery
- **Memory usage**: Similar to nom

### Optimization Opportunities

1. **Token caching** for repeated parsing
2. **State precomputation** for known configurations
3. **Lazy evaluation** of complex combinators

## Testing Strategy

### Unit Tests

```rust
// tests/chumsky_parser_tests.rs
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_global_option_parsing() {
        let parser = ChumskyParser::new(None);
        let result = parser.parse_global_options_only("--ottofile=test.yml --verbose");
        
        assert!(result.is_ok());
        let (options, remaining) = result.unwrap();
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].name, "ottofile");
        assert_eq!(options[0].value, Some("test.yml".to_string()));
    }
    
    #[test]
    fn test_context_aware_disambiguation() {
        let config = create_test_config_with_tasks(&["build", "test"]);
        let parser = ChumskyParser::new(Some(config));
        
        // --verbose build should parse as flag + task
        let result = parser.parse("--verbose build");
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.tasks.len(), 1);
        assert_eq!(parsed.tasks[0].name, "build");
        // Global verbose flag should be set
        assert!(parsed.global_options.verbose);
    }
}
```

### Integration Tests

```rust
// tests/integration_tests.rs
#[test]
fn test_complex_command_parsing() {
    let config = load_test_config("examples/complex.yml");
    let parser = ChumskyParser::new(Some(config));
    
    let input = "otto --ottofile=custom.yml --jobs=4 test --unit --integration build --release";
    let result = parser.parse(input);
    
    assert!(result.is_ok());
    let parsed = result.unwrap();
    
    // Verify global options
    assert_eq!(parsed.global_options.ottofile, Some("custom.yml".into()));
    assert_eq!(parsed.global_options.jobs, Some(4));
    
    // Verify tasks
    assert_eq!(parsed.tasks.len(), 2);
    assert_eq!(parsed.tasks[0].name, "test");
    assert_eq!(parsed.tasks[1].name, "build");
}
```

### Property-Based Testing

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_parser_never_panics(input in ".*") {
        let parser = ChumskyParser::new(None);
        let _ = parser.parse(&input); // Should never panic
    }
    
    #[test]
    fn test_global_options_order_independence(
        ottofile in prop::option::of("[a-z]+\\.yml"),
        jobs in prop::option::of(1u32..10),
        verbose in any::<bool>(),
    ) {
        // Test that global options work in any order
        let parser = ChumskyParser::new(None);
        
        let mut options = Vec::new();
        if let Some(ref file) = ottofile {
            options.push(format!("--ottofile={}", file));
        }
        if let Some(j) = jobs {
            options.push(format!("--jobs={}", j));
        }
        if verbose {
            options.push("--verbose".to_string());
        }
        
        // Test all permutations
        for perm in options.iter().permutations(options.len()) {
            let input = perm.join(" ");
            let result = parser.parse_global_options_only(&input);
            prop_assert!(result.is_ok());
        }
    }
}
```

## Examples

### Basic Usage

```rust
// examples/basic_parsing.rs
use otto::cli::chumsky::ChumskyParser;

fn main() {
    let parser = ChumskyParser::new(None);
    
    // Parse global options only
    let (globals, remaining) = parser
        .parse_global_options_only("--ottofile=build.yml --verbose test --coverage")
        .unwrap();
    
    println!("Global options: {:?}", globals);
    println!("Remaining: {}", remaining);
    
    // Load config and parse tasks
    let config = load_config(globals.ottofile).unwrap();
    let parser = ChumskyParser::new(Some(config));
    let tasks = parser.parse_tasks_only(&remaining).unwrap();
    
    println!("Tasks: {:?}", tasks);
}
```

### Context-Aware Parsing

```rust
// examples/context_aware.rs
use otto::cli::chumsky::ChumskyParser;

fn main() {
    let config = ConfigSpec {
        tasks: {
            let mut tasks = HashMap::new();
            tasks.insert("build".to_string(), TaskSpec { /* ... */ });
            tasks.insert("test".to_string(), TaskSpec { /* ... */ });
            tasks
        },
        // ...
    };
    
    let parser = ChumskyParser::new(Some(config));
    
    // This will parse differently based on config
    let examples = vec![
        "--verbose build",     // --verbose flag + build task
        "--verbose hello",     // --verbose=hello (if hello not a task)
        "--flag=value",        // Always --flag=value
        "--flag taskname",     // Context-dependent
    ];
    
    for example in examples {
        match parser.parse(example) {
            Ok(parsed) => println!("{} → {:?}", example, parsed),
            Err(e) => println!("{} → Error: {}", example, e),
        }
    }
}
```

### Error Handling

```rust
// examples/error_handling.rs
use otto::cli::chumsky::{ChumskyParser, OttoParseError};

fn main() {
    let parser = ChumskyParser::new(None);
    
    let invalid_inputs = vec![
        "otto --unknown-flag",
        "otto biuld",  // typo
        "otto test --invalid-arg",
    ];
    
    for input in invalid_inputs {
        match parser.parse(input) {
            Ok(_) => println!("{} → OK", input),
            Err(OttoParseError::UnknownTask { name, suggestions }) => {
                println!("{} → Unknown task '{}', did you mean: {:?}", input, name, suggestions);
            }
            Err(e) => println!("{} → Error: {}", input, e),
        }
    }
}
```

## Dependencies

### Required Dependencies

```toml
[dependencies]
# Core parsing
chumsky = "0.9"

# Error handling and suggestions
levenshtein = "1.0"
thiserror = "1.0"

# Utilities
itertools = "0.11"

# Existing Otto dependencies
eyre = "0.6"
serde = { version = "1.0", features = ["derive"] }
serde_yaml = "0.9"
```

### Development Dependencies

```toml
[dev-dependencies]
# Testing
criterion = "0.5"
proptest = "1.0"
similar-asserts = "1.4"

# Benchmarking
pprof = { version = "0.11", features = ["criterion", "flamegraph"] }
```

### Feature Flags

```toml
[features]
default = ["chumsky-parser"]
chumsky-parser = []
nom-parser = []  # For backward compatibility during migration
```

## Conclusion

Migrating to Chumsky addresses Otto's core parsing challenges:

1. **Context-sensitive disambiguation** through stateful parsing
2. **Superior error handling** with custom error types and suggestions
3. **Unified architecture** eliminating hybrid nom/bespoke complexity
4. **Maintainable codebase** with clear separation of concerns

The migration can be done incrementally with feature flags, ensuring no disruption to existing functionality while providing a path to a more robust parsing solution.

The key insight is that Otto's grammar is inherently context-sensitive, and Chumsky is specifically designed for this class of problems, making it the ideal choice for this use case. 