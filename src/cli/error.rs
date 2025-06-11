#![allow(unused_imports, unused_variables, unused_attributes, unused_mut, dead_code)]

use std::env;
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::path::PathBuf;

use eyre::{eyre, Result, Report};

// Since these error types aren't used elsewhere in the codebase,
// we can simplify this to just use eyre::Report directly.
// If specific error types are needed later, they can be added back.

pub type OttoResult<T> = Result<T, Report>;

// Helper functions for creating specific Otto errors
pub fn home_undefined_error(source: env::VarError) -> Report {
    eyre!("env var error: {}", source)
}

pub fn canonicalize_error(source: std::io::Error) -> Report {
    eyre!("canonicalize error: {}", source)
}

pub fn divine_error(path: PathBuf) -> Report {
    eyre!("divine error; unable to find ottofile from path=[{}]", path.display())
}

pub fn relative_path_error() -> Report {
    eyre!("relative path error")
}

pub fn current_exe_filename_error() -> Report {
    eyre!("current exe filename error")
}

pub fn config_error(source: Report) -> Report {
    eyre!("config error: {}", source)
}

pub fn clap_error(source: clap::Error) -> Report {
    eyre!("Clap parse error: {}", source)
}

// Keep SilentError as a unit struct since it has special Display behavior
#[derive(Debug)]
pub struct SilentError;

impl Display for SilentError {
    fn fmt(&self, _f: &mut Formatter) -> FmtResult {
        Ok(())
    }
}

impl std::error::Error for SilentError {}