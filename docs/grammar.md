# Otto CLI Grammar Specification

This document defines the formal grammar for the Otto CLI command-line interface using multiple notation systems familiar to programming language experts.

## Table of Contents

1. [Overview](#overview)
2. [EBNF Grammar](#ebnf-grammar)
3. [BNF Grammar](#bnf-grammar)
4. [Lexical Analysis](#lexical-analysis)
5. [Semantic Rules](#semantic-rules)
6. [Grammar Ambiguities](#grammar-ambiguities)
7. [Examples](#examples)

## Overview

Otto CLI uses a two-pass parsing approach:
- **Pass 1**: Extract global options that can appear anywhere in the command line
- **Pass 2**: Parse task invocations with their arguments, validated against configuration

The grammar supports:
- Global options (affecting Otto behavior)
- Built-in commands (`graph`, `help`, `version`)
- Task invocations with typed arguments
- Mixed global and task-specific options

## EBNF Grammar

Extended Backus-Naur Form (ISO/IEC 14977):

```ebnf
(* Otto CLI Grammar *)

command_line = [ global_options ], [ command | task_invocations ] ;

global_options = { global_option } ;

global_option = global_option_long_equals
              | global_option_long_space
              | global_option_short_space
              | global_option_flag ;

global_option_long_equals = "--", identifier, "=", argument_value ;
global_option_long_space  = "--", identifier, whitespace1, argument_value ;
global_option_short_space = "-", short_char, whitespace1, argument_value ;
global_option_flag       = ( "--", identifier ) | ( "-", short_char ) ;

command = "graph", [ graph_options ]
        | "help", [ task_name ]
        | "version" ;

graph_options = { graph_option } ;
graph_option = "--format", whitespace1, graph_format
             | "--output", whitespace1, file_path ;

graph_format = "ascii" | "dot" | "svg" ;

task_invocations = task_invocation, { whitespace1, task_invocation } ;

task_invocation = task_name, { whitespace1, task_argument } ;

task_argument = task_argument_long_equals
              | task_argument_long_space
              | task_argument_short_space
              | task_argument_flag ;

task_argument_long_equals = "--", identifier, "=", argument_value ;
task_argument_long_space  = "--", identifier, whitespace1, argument_value ;
task_argument_short_space = "-", letter, whitespace1, argument_value ;
task_argument_flag       = ( "--", identifier ) | ( "-", letter ) ;

(* Lexical Rules *)

identifier = ( letter | "_" ), { letter | digit | "_" | "-" } ;
task_name = identifier ;
argument_value = quoted_string | unquoted_value ;
quoted_string = '"', { ? any character except '"' ? }, '"'
              | "'", { ? any character except "'" ? }, "'" ;
unquoted_value = { ? any non-whitespace character ? } ;

short_char = "o" | "a" | "j" | "H" | "t" | "v" | "h" | "V" ;
letter = "a" | "b" | "c" | ? ... ? | "z" | "A" | "B" | ? ... ? | "Z" ;
digit = "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" ;
whitespace1 = { " " | "\t" | "\n" | "\r" } ;
file_path = argument_value ;
```

## BNF Grammar

Classic Backus-Naur Form:

```bnf
<command_line> ::= <global_options> <command>
                 | <global_options> <task_invocations>
                 | <global_options>
                 | <command>
                 | <task_invocations>
                 | <empty>

<global_options> ::= <global_option>
                   | <global_options> <global_option>
                   | <empty>

<global_option> ::= <global_option_long_equals>
                  | <global_option_long_space>
                  | <global_option_short_space>
                  | <global_option_flag>

<global_option_long_equals> ::= "--" <global_option_name> "=" <argument_value>
<global_option_long_space>  ::= "--" <global_option_name> <whitespace1> <argument_value>
<global_option_short_space> ::= "-" <global_short_char> <whitespace1> <argument_value>
<global_option_flag>       ::= "--" <global_flag_name>
                             | "-" <global_short_flag>

<global_option_name> ::= "ottofile" | "api" | "jobs" | "home" | "tasks" | "verbosity"
<global_flag_name>   ::= "help" | "version" | "verbose"
<global_short_char>  ::= "o" | "a" | "j" | "H" | "t" | "v"
<global_short_flag>  ::= "h" | "V"

<command> ::= <graph_command>
            | <help_command>
            | <version_command>

<graph_command> ::= "graph" <graph_options>
<help_command>  ::= "help" <task_name>
                  | "help"
<version_command> ::= "version"

<graph_options> ::= <graph_option>
                  | <graph_options> <graph_option>
                  | <empty>

<graph_option> ::= "--format" <whitespace1> <graph_format>
                 | "--output" <whitespace1> <file_path>

<graph_format> ::= "ascii" | "dot" | "svg"

<task_invocations> ::= <task_invocation>
                     | <task_invocations> <whitespace1> <task_invocation>

<task_invocation> ::= <task_name> <task_arguments>

<task_arguments> ::= <task_argument>
                   | <task_arguments> <whitespace1> <task_argument>
                   | <empty>

<task_argument> ::= <task_argument_long_equals>
                  | <task_argument_long_space>
                  | <task_argument_short_space>
                  | <task_argument_flag>

<task_argument_long_equals> ::= "--" <identifier> "=" <argument_value>
<task_argument_long_space>  ::= "--" <identifier> <whitespace1> <argument_value>
<task_argument_short_space> ::= "-" <letter> <whitespace1> <argument_value>
<task_argument_flag>       ::= "--" <identifier>
                             | "-" <letter>

<task_name>      ::= <identifier>
<identifier>     ::= <id_start> <id_continue>
<id_start>       ::= <letter> | "_"
<id_continue>    ::= <id_char>
                   | <id_continue> <id_char>
                   | <empty>
<id_char>        ::= <letter> | <digit> | "_" | "-"

<argument_value> ::= <quoted_string> | <unquoted_value>
<quoted_string>  ::= '"' <string_content> '"'
                   | "'" <string_content> "'"
<string_content> ::= <string_char>
                   | <string_content> <string_char>
                   | <empty>
<string_char>    ::= <any_char_except_quote>
<unquoted_value> ::= <nonws_char>
                   | <unquoted_value> <nonws_char>

<letter>         ::= "a" | "b" | ... | "z" | "A" | "B" | ... | "Z"
<digit>          ::= "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
<whitespace1>    ::= <ws_char>
                   | <whitespace1> <ws_char>
<ws_char>        ::= " " | "\t" | "\n" | "\r"
<nonws_char>     ::= <any_char_except_whitespace>
<file_path>      ::= <argument_value>
<empty>          ::= ""
```

## Lexical Analysis

### Token Types

```rust
// Terminal symbols (tokens)
enum Token {
    // Literals
    Identifier(String),
    QuotedString(String),
    UnquotedValue(String),

    // Keywords
    Graph,
    Help,
    Version,

    // Operators
    DoubleDash,      // "--"
    SingleDash,      // "-"
    Equals,          // "="

    // Whitespace
    Whitespace,

    // Special
    EOF,
}
```

### Lexical Rules

```
IDENTIFIER     ::= [a-zA-Z_][a-zA-Z0-9_-]*
QUOTED_STRING  ::= "([^"]*)" | '([^']*)'
UNQUOTED_VALUE ::= [^\s]+
WHITESPACE     ::= [\s]+
DOUBLE_DASH    ::= "--"
SINGLE_DASH    ::= "-"
EQUALS         ::= "="
```

### Tokenization Order

1. **Whitespace** (consumed, not returned)
2. **Keywords** (`graph`, `help`, `version`)
3. **Operators** (`--`, `-`, `=`)
4. **Quoted strings** (higher precedence than unquoted)
5. **Identifiers** (alphanumeric + underscore + dash)
6. **Unquoted values** (fallback for non-whitespace)

## Semantic Rules

### Global Options

| Option | Short | Type | Description |
|--------|-------|------|-------------|
| `--ottofile` | `-o` | `String` | Path to Otto configuration file |
| `--api` | `-a` | `String` | API URL |
| `--jobs` | `-j` | `u32` | Number of parallel jobs |
| `--home` | `-H` | `String` | Otto home directory |
| `--tasks` | `-t` | `String` | Comma-separated task list |
| `--verbosity` | `-v` | `u8` | Verbosity level (0-9) |
| `--help` | `-h` | `Flag` | Show help message |
| `--version` | `-V` | `Flag` | Show version |
| `--verbose` | | `Flag` | Enable verbose output |

### Task Arguments

Task arguments are dynamically typed based on configuration:

```rust
enum ValidatedValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Path(PathBuf),
    Url(String),
}
```

### Parameter Types

```rust
enum ParamType {
    FLG,  // Flag (boolean, no value)
    OPT,  // Optional (requires value)
    POS,  // Positional (requires value)
}
```

### Validation Rules

1. **Global options** are validated against a fixed schema
2. **Task arguments** are validated against dynamic configuration
3. **Short flags** are mapped to long names via configuration
4. **Type coercion** follows parameter specifications
5. **Default values** are applied for missing optional parameters
6. **Required parameters** must be provided or have defaults

## Grammar Ambiguities

### Inherent Ambiguities

The grammar contains intentional ambiguities that are resolved through precedence:

#### 1. Global vs Task Arguments

```bash
otto --jobs 4 build
```

**Ambiguous interpretations:**
- `--jobs` is a global option with value `4`, `build` is a task
- `--jobs` is a task argument for task `4` with value `build`

**Resolution:** Global options are parsed first (Pass 1), remaining tokens go to task parsing (Pass 2).

#### 2. Flag vs Argument with Space

```bash
otto --verbose build
```

**Ambiguous interpretations:**
- `--verbose` is a flag, `build` is a task name
- `--verbose` is an argument with value `build`

**Resolution:** Known global flags are recognized by name, unknown flags are treated as arguments with values.

#### 3. Task Argument Order

```bash
otto build --flag value
```

**Ambiguous interpretations:**
- `--flag` is a boolean flag, `value` is a positional argument
- `--flag` is an argument with value `value`

**Resolution:** Arguments with values are parsed before flags in the `alt()` combinator.

### Disambiguation Strategy

```rust
// Parser precedence (highest to lowest)
task_argument = alt((
    task_argument_long_with_equals,  // --arg=value (highest)
    task_argument_long_with_space,   // --arg value
    task_argument_short_with_space,  // -a value
    task_argument_flag,              // --flag (lowest)
))
```

## Examples

### Basic Task Invocation

```bash
otto build
```

**Parse tree:**
```
command_line
├── global_options: []
└── task_invocations
    └── task_invocation
        ├── task_name: "build"
        └── task_arguments: []
```

### Global Options with Task

```bash
otto --ottofile custom.yml --jobs 4 build --verbose
```

**Parse tree:**
```
command_line
├── global_options
│   ├── global_option: ottofile="custom.yml"
│   └── global_option: jobs=4
└── task_invocations
    └── task_invocation
        ├── task_name: "build"
        └── task_arguments
            └── task_argument: verbose=flag
```

### Multiple Tasks with Arguments

```bash
otto test --coverage build --release deploy --env production
```

**Parse tree:**
```
command_line
├── global_options: []
└── task_invocations
    ├── task_invocation
    │   ├── task_name: "test"
    │   └── task_arguments
    │       └── task_argument: coverage=flag
    ├── task_invocation
    │   ├── task_name: "build"
    │   └── task_arguments
    │       └── task_argument: release=flag
    └── task_invocation
        ├── task_name: "deploy"
        └── task_arguments
            └── task_argument: env="production"
```

### Graph Command

```bash
otto graph --format dot --output graph.dot
```

**Parse tree:**
```
command_line
├── global_options: []
└── command
    └── graph_command
        ├── keyword: "graph"
        └── graph_options
            ├── graph_option: format="dot"
            └── graph_option: output="graph.dot"
```

### Complex Mixed Example

```bash
otto --ottofile=build.yml --verbose test --unit --integration build --release=true
```

**Parse tree:**
```
command_line
├── global_options
│   ├── global_option: ottofile="build.yml"
│   └── global_option: verbose=flag
└── task_invocations
    ├── task_invocation
    │   ├── task_name: "test"
    │   └── task_arguments
    │       ├── task_argument: unit=flag
    │       └── task_argument: integration=flag
    └── task_invocation
        ├── task_name: "build"
        └── task_arguments
            └── task_argument: release="true"
```

## Implementation Notes

### Two-Pass Parsing

1. **Pass 1**: `parse_global_options_only()`
   - Extracts known global options from anywhere in command line
   - Returns `(Vec<GlobalOption>, remaining_args: String)`
   - Uses hybrid approach: nom for individual parsing, loop for structure

2. **Pass 2**: `parse_tasks_only()`
   - Parses remaining arguments as task invocations
   - Validates against loaded configuration
   - Pure nom parser combinators

### Error Handling

```rust
enum ParseError {
    // nom errors
    NomError { input: String, position: usize, kind: ErrorKind, context: Vec<String> },

    // Semantic errors
    UnknownTask { name: String, suggestions: Vec<String> },
    UnknownGlobalOption { name: String },
    UnknownTaskArgument { task_name: String, arg_name: String },

    // Validation errors
    InvalidArgumentValue { arg_name: String, value: String, expected: String },
    ValidationError { task_name: String, arg_name: String, error: String },

    // Input errors
    UnconsumedInput { remaining: String },
    IncompleteInput,
}
```

### Grammar Extensions

The grammar is designed to be extensible:

1. **New global options** can be added to the known options list
2. **New commands** can be added to the command alternatives
3. **Task arguments** are dynamically validated against configuration
4. **Parameter types** can be extended in the validation system

This grammar specification provides a formal foundation for understanding, implementing, and extending the Otto CLI parser.
