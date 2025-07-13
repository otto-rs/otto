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

// Internal parsing types
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalOption {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskInvocation {
    pub name: String,
    pub arguments: Vec<TaskArgument>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskArgument {
    pub name: String,
    pub value: Option<String>,
}

// Raw parsed command before validation
#[derive(Debug, Clone, PartialEq)]
pub struct RawParsedCommand {
    pub global_options: Vec<GlobalOption>,
    pub tasks: Vec<TaskInvocation>,
}
