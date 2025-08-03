//#![allow(unused_imports, unused_variables, dead_code)]

use eyre::Result;
use serde::de::{Deserializer, MapAccess, Visitor, IgnoredAny};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::vec::Vec;

use crate::cfg::param::{deserialize_param_map, ParamSpecs};

pub type TaskSpecs = HashMap<String, TaskSpec>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionSpec {
    Bash(String),
    Python(String),
}

impl fmt::Display for ActionSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionSpec::Bash(_) => write!(f, "bash action"),
            ActionSpec::Python(_) => write!(f, "python action"),
        }
    }
}

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

    #[serde(flatten)]
    pub action: Option<ActionSpec>,
}

/// Reusable function to process script content by trimming common indentation
fn deserialize_script_content<E>(s: String) -> Result<String, E>
where
    E: serde::de::Error,
{
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



impl<'de> Deserialize<'de> for ActionSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ActionVisitor;

        impl<'de> Visitor<'de> for ActionVisitor {
            type Value = ActionSpec;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("either 'bash' or 'python' key with script content")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut bash_content: Option<String> = None;
                let mut python_content: Option<String> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "bash" => {
                            if bash_content.is_some() {
                                return Err(serde::de::Error::duplicate_field("bash"));
                            }
                            let content: String = map.next_value()?;
                            bash_content = Some(deserialize_script_content(content)?);
                        }
                        "python" => {
                            if python_content.is_some() {
                                return Err(serde::de::Error::duplicate_field("python"));
                            }
                            let content: String = map.next_value()?;
                            python_content = Some(deserialize_script_content(content)?);
                        }
                        _ => {
                            // Skip unknown fields
                            let _: IgnoredAny = map.next_value()?;
                        }
                    }
                }

                match (bash_content, python_content) {
                    (Some(bash), None) => Ok(ActionSpec::Bash(bash)),
                    (None, Some(python)) => Ok(ActionSpec::Python(python)),
                    (Some(_), Some(_)) => Err(serde::de::Error::custom(
                        "task cannot have both 'bash' and 'python' actions"
                    )),
                    (None, None) => Err(serde::de::Error::custom(
                        "task must have either 'bash' or 'python' action"
                    )),
                }
            }
        }

        deserializer.deserialize_map(ActionVisitor)
    }
}

impl TaskSpec {
    pub fn validate(&self) -> Result<()> {
        match &self.action {
            Some(ActionSpec::Bash(content)) if content.trim().is_empty() => {
                Err(eyre::eyre!("Bash action cannot be empty"))
            }
            Some(ActionSpec::Python(content)) if content.trim().is_empty() => {
                Err(eyre::eyre!("Python action cannot be empty"))
            }
            None => Err(eyre::eyre!("Task must have either 'bash' or 'python' action")),
            _ => Ok(())
        }
    }

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
        action: Option<ActionSpec>,
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
