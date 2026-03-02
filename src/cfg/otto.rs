//#![allow(unused_imports, unused_variables, dead_code)]

use serde::{Deserialize, Serialize};
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

fn default_keep_days() -> u64 {
    30
}

fn default_keep_last() -> usize {
    10
}

fn default_keep_failed() -> u64 {
    60
}

fn default_auto_prune() -> bool {
    true
}

fn default_prune_interval_hours() -> u64 {
    24
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RetentionSpec {
    /// Delete runs older than this many days (default: 30)
    #[serde(default = "default_keep_days")]
    pub keep_days: u64,

    /// Always keep at least this many most recent runs (default: 10)
    #[serde(default = "default_keep_last")]
    pub keep_last: usize,

    /// Keep failed runs for this many days (default: 60)
    #[serde(default = "default_keep_failed")]
    pub keep_failed: u64,

    /// Enable automatic pruning after runs (default: true)
    #[serde(default = "default_auto_prune")]
    pub auto_prune: bool,

    /// Minimum hours between auto-prune runs (default: 24)
    #[serde(default = "default_prune_interval_hours")]
    pub prune_interval_hours: u64,
}

impl Default for RetentionSpec {
    fn default() -> Self {
        Self {
            keep_days: default_keep_days(),
            keep_last: default_keep_last(),
            keep_failed: default_keep_failed(),
            auto_prune: default_auto_prune(),
            prune_interval_hours: default_prune_interval_hours(),
        }
    }
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
        retention: RetentionSpec::default(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
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

    #[serde(default)]
    pub retention: RetentionSpec,
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
            retention: RetentionSpec::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retention_spec_defaults() {
        let spec = RetentionSpec::default();
        assert_eq!(spec.keep_days, 30);
        assert_eq!(spec.keep_last, 10);
        assert_eq!(spec.keep_failed, 60);
        assert!(spec.auto_prune);
        assert_eq!(spec.prune_interval_hours, 24);
    }

    #[test]
    fn test_retention_spec_deserialize_empty() {
        let yaml = "{}";
        let spec: RetentionSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec, RetentionSpec::default());
    }

    #[test]
    fn test_retention_spec_deserialize_partial() {
        let yaml = "keep_days: 14\nkeep_last: 5";
        let spec: RetentionSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.keep_days, 14);
        assert_eq!(spec.keep_last, 5);
        assert_eq!(spec.keep_failed, 60); // default
        assert!(spec.auto_prune); // default
        assert_eq!(spec.prune_interval_hours, 24); // default
    }

    #[test]
    fn test_retention_spec_deserialize_full() {
        let yaml = r#"
keep_days: 7
keep_last: 3
keep_failed: 14
auto_prune: false
prune_interval_hours: 12
"#;
        let spec: RetentionSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.keep_days, 7);
        assert_eq!(spec.keep_last, 3);
        assert_eq!(spec.keep_failed, 14);
        assert!(!spec.auto_prune);
        assert_eq!(spec.prune_interval_hours, 12);
    }

    #[test]
    fn test_otto_spec_with_retention() {
        let yaml = r#"
name: test-project
retention:
  keep_days: 14
  keep_last: 5
"#;
        let spec: OttoSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.name, "test-project");
        assert_eq!(spec.retention.keep_days, 14);
        assert_eq!(spec.retention.keep_last, 5);
        assert_eq!(spec.retention.keep_failed, 60); // default
    }

    #[test]
    fn test_otto_spec_without_retention() {
        let yaml = "name: test-project";
        let spec: OttoSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.retention, RetentionSpec::default());
    }

    #[test]
    fn test_retention_spec_roundtrip() {
        let spec = RetentionSpec {
            keep_days: 7,
            keep_last: 3,
            keep_failed: 14,
            auto_prune: false,
            prune_interval_hours: 12,
        };
        let yaml = serde_yaml::to_string(&spec).unwrap();
        let deserialized: RetentionSpec = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(spec, deserialized);
    }
}
