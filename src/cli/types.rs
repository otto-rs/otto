use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCommand {
    pub global_options: GlobalOptions,
    pub tasks: Vec<ParsedTask>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTask {
    pub name: String,
    pub arguments: HashMap<String, ValidatedValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlobalOptions {
    pub ottofile: Option<PathBuf>,
    pub api: Option<String>,
    pub jobs: Option<u32>,
    pub home: Option<PathBuf>,
    pub tasks: Option<String>,
    pub verbosity: Option<u8>,
    pub timeout: Option<u64>,
    pub help: bool,
    pub version: bool,
}

impl Default for GlobalOptions {
    fn default() -> Self {
        Self {
            ottofile: None,
            api: None,
            jobs: None,
            home: None,
            tasks: None,
            verbosity: None,
            timeout: None,
            help: false,
            version: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValidatedValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Path(PathBuf),
    Url(String),
}

impl ValidatedValue {
    pub fn as_string(&self) -> Option<&str> {
        match self {
            ValidatedValue::String(s) => Some(s),
            ValidatedValue::Url(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            ValidatedValue::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            ValidatedValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_path(&self) -> Option<&PathBuf> {
        match self {
            ValidatedValue::Path(p) => Some(p),
            _ => None,
        }
    }
}

// Token types for parsing
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    TaskName(String),
    GlobalOption { name: String, value: Option<String> },
    TaskArgument { name: String, value: Option<String> },
    Help,
    Version,
    Unknown(String),
}

// Parsing state for completion
#[derive(Debug, Clone)]
pub enum ParseState {
    ExpectingGlobalOption,
    ExpectingTaskName,
    ExpectingTaskArgument { task_name: String },
    ExpectingArgumentValue { task_name: String, arg_name: String },
}

#[derive(Debug, Clone)]
pub struct PartialParseResult {
    pub state: ParseState,
    pub tokens: Vec<String>,
}
