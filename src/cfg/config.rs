//#![allow(unused_imports, unused_variables, dead_code)]

use serde::Deserialize;

pub use crate::cfg::otto::{OttoSpec, default_otto};
pub use crate::cfg::param::{ParamSpec, ParamSpecs, Value};
pub use crate::cfg::task::{TaskSpec, TaskSpecs, deserialize_task_map};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ConfigSpec {
    #[serde(default = "default_otto")]
    pub otto: OttoSpec,

    #[serde(default, deserialize_with = "deserialize_task_map")]
    pub tasks: TaskSpecs,
}

impl Default for ConfigSpec {
    fn default() -> Self {
        Self {
            otto: default_otto(),
            tasks: TaskSpecs::new(),
        }
    }
}
