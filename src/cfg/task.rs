//#![allow(unused_imports, unused_variables, dead_code)]

use eyre::Result;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::vec::Vec;

use crate::cfg::param::{deserialize_param_map, ParamSpecs};

pub type TaskSpecs = HashMap<String, TaskSpec>;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct TaskSpec {
    #[serde(skip_deserializing)]
    pub name: String,

    #[serde(default)]
    pub help: Option<String>,

    #[serde(default)]
    pub after: Vec<String>,

    #[serde(default)]
    pub before: Vec<String>,

    #[serde(default)]
    pub input: Vec<String>,

    #[serde(default)]
    pub output: Vec<String>,

    #[serde(default)]
    pub envs: HashMap<String, String>,

    #[serde(default, deserialize_with = "deserialize_param_map")]
    pub params: ParamSpecs,

    #[serde(default, deserialize_with = "deserialize_script")]
    pub action: String,
}

fn deserialize_script<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    // For block scalars, preserve the exact content but trim any common indentation
    let lines: Vec<&str> = s.lines().collect();

    // Find minimum indentation (ignoring empty lines)
    let min_indent = lines.iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    // Remove common indentation from each line
    let dedented: Vec<String> = lines.iter()
        .map(|line| {
            if line.len() > min_indent {
                line[min_indent..].to_string()
            } else {
                line.to_string()
            }
        })
        .collect();

    // Join lines and trim any leading/trailing empty lines
    let result = dedented.join("\n");
    Ok(result.trim_start().trim_end().to_string())
}

impl TaskSpec {
    #[must_use]
    pub fn new(
        name: String,
        help: Option<String>,
        after: Vec<String>,
        before: Vec<String>,
        input: Vec<String>,
        output: Vec<String>,
        envs: HashMap<String, String>,
        params: ParamSpecs,
        action: String,
    ) -> Self {
        Self {
            name,
            help,
            after,
            before,
            input,
            output,
            envs,
            params,
            action,
        }
    }
}

fn namify(name: &str) -> String {
    name.split('|')
        .find(|&part| part.starts_with("--"))
        .map_or_else(|| name.split('|').next().unwrap().trim_start_matches('-').to_string(), |s| s.trim_start_matches("--").to_string())
}

#[test]
fn test_namify() {
    assert_eq!(namify("-g|--greeting"), "greeting".to_string());
    assert_eq!(namify("-k"), "k".to_string());
    assert_eq!(namify("--name"), "name".to_string());
}

pub fn deserialize_task_map<'de, D>(deserializer: D) -> Result<TaskSpecs, D::Error>
where
    D: Deserializer<'de>,
{
    struct TaskMap;

    impl<'de> Visitor<'de> for TaskMap {
        type Value = TaskSpecs;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map of name to Task")
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut tasks = TaskSpecs::new();
            while let Some((name, mut task_spec)) = map.next_entry::<String, TaskSpec>()? {
                task_spec.name = namify(&name);
                tasks.insert(name.clone(), task_spec);
            }
            Ok(tasks)
        }
    }
    deserializer.deserialize_map(TaskMap)
}
