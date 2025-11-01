#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::fmt;
use std::io;

use eyre::{Report, Result, eyre};
use std::fmt::{Debug, Display, Formatter};

// Since ConfigError types aren't used elsewhere in the codebase,
// we can simplify this to just use eyre::Report directly.
// If specific error types are needed later, they can be added back.

pub type ConfigResult<T> = Result<T, Report>;

// Helper functions for creating specific config errors
pub fn config_load_error(source: std::io::Error) -> Report {
    eyre!("config load error: {}", source)
}

pub fn serde_yaml_error(source: serde_yaml::Error) -> Report {
    eyre!("serde yaml error: {}", source)
}

/*
// These error variants were commented out and unused
#[error("flag lookup error; flag={0} not found")]
FlagLookupError(String),
#[error("name lookup error; name={0} not found")]
NameLookupError(String),
*/
