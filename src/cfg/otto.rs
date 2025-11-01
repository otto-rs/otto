//#![allow(unused_imports, unused_variables, dead_code)]

use serde::Deserialize;
use std::collections::HashMap;
use std::vec::Vec;

fn default_name() -> String {
    "otto".to_string()
}

fn default_about() -> String {
    "A task runner".to_string()
}

fn default_api() -> String {
    "1".to_string()
}

fn default_jobs() -> usize {
    num_cpus::get()
}

fn default_home() -> String {
    "~/.otto".to_string()
}

fn default_tasks() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_verbosity() -> u8 {
    1
}

#[must_use]
pub fn default_otto() -> OttoSpec {
    OttoSpec {
        name: default_name(),
        about: default_about(),
        api: default_api(),
        jobs: default_jobs(),
        home: default_home(),
        tasks: default_tasks(),
        verbosity: default_verbosity(),
        envs: HashMap::new(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct OttoSpec {
    #[serde(default = "default_name")]
    pub name: String,

    #[serde(default = "default_about")]
    pub about: String,

    #[serde(default = "default_api")]
    pub api: String,

    #[serde(default = "default_jobs")]
    pub jobs: usize,

    #[serde(default = "default_home")]
    pub home: String,

    #[serde(default = "default_tasks")]
    pub tasks: Vec<String>,

    #[serde(default = "default_verbosity")]
    pub verbosity: u8,

    #[serde(default)]
    pub envs: HashMap<String, String>,
}

impl Default for OttoSpec {
    fn default() -> Self {
        Self {
            name: default_name(),
            about: default_about(),
            api: default_api(),
            jobs: default_jobs(),
            home: default_home(),
            tasks: default_tasks(),
            verbosity: default_verbosity(),
            envs: HashMap::new(),
        }
    }
}
