//#![allow(unused_imports, unused_variables, dead_code)]

use eyre::Result;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::vec::Vec;

use crate::cfg::param::{ParamSpecs, deserialize_param_map};

pub type TaskSpecs = HashMap<String, TaskSpec>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskSpec {
    pub name: String,
    pub help: Option<String>,
    pub after: Vec<String>,
    pub before: Vec<String>,
    pub input: Vec<String>,
    pub output: Vec<String>,
    pub envs: HashMap<String, String>,
    pub params: ParamSpecs,
    pub action: String,
    pub interactive: Option<bool>,
}

// Helper struct for deserialization that accepts bash:, python:, or action: fields
#[derive(Debug, Deserialize)]
struct TaskSpecHelper {
    #[serde(default)]
    help: Option<String>,

    #[serde(default)]
    after: Vec<String>,

    #[serde(default)]
    before: Vec<String>,

    #[serde(default)]
    input: Vec<String>,

    #[serde(default)]
    output: Vec<String>,

    #[serde(default)]
    envs: HashMap<String, String>,

    #[serde(default, deserialize_with = "deserialize_param_map")]
    params: ParamSpecs,

    // Support for new bash: field
    #[serde(default)]
    bash: Option<String>,

    // Support for new python: field
    #[serde(default)]
    python: Option<String>,

    // Legacy support for action: field (deprecated)
    #[serde(default)]
    action: Option<String>,

    // Support for interactive tasks
    #[serde(default)]
    interactive: Option<bool>,
}

impl<'de> Deserialize<'de> for TaskSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let helper = TaskSpecHelper::deserialize(deserializer)?;

        let action = if let Some(bash_script) = helper.bash {
            let bash_script = deserialize_script_string(&bash_script);
            if bash_script.trim_start().starts_with("#!") {
                bash_script
            } else {
                format!("#!/bin/bash\n{bash_script}")
            }
        } else if let Some(python_script) = helper.python {
            let python_script = deserialize_script_string(&python_script);
            if python_script.trim_start().starts_with("#!") {
                python_script
            } else {
                format!("#!/usr/bin/env python3\n{python_script}")
            }
        } else if let Some(action_script) = helper.action {
            deserialize_script_string(&action_script)
        } else {
            String::new()
        };

        Ok(TaskSpec {
            name: String::new(), // Will be set by deserialize_task_map
            help: helper.help,
            after: helper.after,
            before: helper.before,
            input: helper.input,
            output: helper.output,
            envs: helper.envs,
            params: helper.params,
            action,
            interactive: helper.interactive,
        })
    }
}

impl Serialize for TaskSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;

        if let Some(ref help) = self.help {
            map.serialize_entry("help", help)?;
        }

        if !self.after.is_empty() {
            map.serialize_entry("after", &self.after)?;
        }

        if !self.before.is_empty() {
            map.serialize_entry("before", &self.before)?;
        }

        if !self.input.is_empty() {
            map.serialize_entry("input", &self.input)?;
        }

        if !self.output.is_empty() {
            map.serialize_entry("output", &self.output)?;
        }

        if !self.envs.is_empty() {
            map.serialize_entry("envs", &self.envs)?;
        }

        if !self.params.is_empty() {
            map.serialize_entry("params", &self.params)?;
        }

        if let Some(interactive) = self.interactive {
            map.serialize_entry("interactive", &interactive)?;
        }

        // Serialize action as "bash:" if it starts with #!/bin/bash
        if !self.action.is_empty() {
            if self.action.trim_start().starts_with("#!/bin/bash") {
                let bash_script = self
                    .action
                    .trim_start()
                    .strip_prefix("#!/bin/bash\n")
                    .or_else(|| self.action.trim_start().strip_prefix("#!/bin/bash"))
                    .unwrap_or(&self.action);
                map.serialize_entry("bash", bash_script)?;
            } else if self.action.trim_start().starts_with("#!/usr/bin/env python3") {
                let python_script = self
                    .action
                    .trim_start()
                    .strip_prefix("#!/usr/bin/env python3\n")
                    .or_else(|| self.action.trim_start().strip_prefix("#!/usr/bin/env python3"))
                    .unwrap_or(&self.action);
                map.serialize_entry("python", python_script)?;
            } else {
                map.serialize_entry("action", &self.action)?;
            }
        }

        map.end()
    }
}

fn deserialize_script_string(s: &str) -> String {
    // For block scalars, preserve the exact content but trim any common indentation
    let lines: Vec<&str> = s.lines().collect();

    // Find minimum indentation (ignoring empty lines)
    let min_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    let dedented: Vec<String> = lines
        .iter()
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
    result.trim_start().trim_end().to_string()
}

impl TaskSpec {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
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
        interactive: Option<bool>,
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
            interactive,
        }
    }
}

fn namify(name: &str) -> String {
    name.split('|').find(|&part| part.starts_with("--")).map_or_else(
        || name.split('|').next().unwrap().trim_start_matches('-').to_string(),
        |s| s.trim_start_matches("--").to_string(),
    )
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
