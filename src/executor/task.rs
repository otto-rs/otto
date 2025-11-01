use daggy::Dag;
use eyre::Result;
use hex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::cfg::config::Value;
use crate::cfg::env as env_eval;
use crate::cfg::task::TaskSpec;

pub type DAG<T> = Dag<T, (), u32>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub name: String,
    pub task_deps: Vec<String>,
    pub file_deps: Vec<String>,
    pub output_deps: Vec<String>,
    pub envs: HashMap<String, String>,
    pub values: HashMap<String, Value>,
    pub action: String,
    pub hash: String,
}

impl Task {
    #[must_use]
    pub fn new(
        name: String,
        task_deps: Vec<String>,
        file_deps: Vec<String>,
        output_deps: Vec<String>,
        envs: HashMap<String, String>,
        values: HashMap<String, Value>,
        action: String,
    ) -> Self {
        let hash = calculate_hash(&action);
        Self {
            name,
            task_deps,
            file_deps,
            output_deps,
            envs,
            values,
            action,
            hash,
        }
    }

    #[must_use]
    pub fn from_task(task_spec: &TaskSpec) -> Self {
        let _name = task_spec.name.clone();
        let _task_deps = task_spec.before.clone();

        // Get current working directory for glob resolution
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        Self::from_task_with_cwd_and_global_envs(task_spec, &cwd, &HashMap::new())
    }

    #[must_use]
    pub fn from_task_with_cwd(task_spec: &TaskSpec, cwd: &std::path::Path) -> Self {
        Self::from_task_with_cwd_and_global_envs(task_spec, cwd, &HashMap::new())
    }

    #[must_use]
    pub fn from_task_with_cwd_and_global_envs(
        task_spec: &TaskSpec,
        cwd: &std::path::Path,
        global_envs: &HashMap<String, String>,
    ) -> Self {
        let name = task_spec.name.clone();
        let task_deps = task_spec.before.clone();

        // Resolve file globs from input to canonical paths using explicit cwd
        let file_deps = Self::resolve_file_globs(&task_spec.input, cwd);

        // Resolve output globs to canonical paths using explicit cwd
        let output_deps = Self::resolve_file_globs(&task_spec.output, cwd);

        // Evaluate environment variables with two-level merging: global then task-level
        let evaluated_envs = Self::evaluate_merged_envs(global_envs, &task_spec.envs, cwd).unwrap_or_else(|e| {
            eprintln!("Warning: Failed to evaluate environment variables for task '{name}': {e}");
            HashMap::new()
        });

        // Note: We do NOT add after tasks here since they depend on us, not vice versa
        // The after dependencies will be handled during DAG construction
        let values = HashMap::new();
        let action = task_spec.action.trim().to_string(); // Trim whitespace from script content
        Self::new(name, task_deps, file_deps, output_deps, evaluated_envs, values, action)
    }

    /// Evaluate and merge environment variables from global and task-level sources
    fn evaluate_merged_envs(
        global_envs: &HashMap<String, String>,
        task_envs: &HashMap<String, String>,
        working_dir: &std::path::Path,
    ) -> Result<HashMap<String, String>> {
        // Step 1: Create merged environment for task evaluation (global + task)
        let mut merged_envs = global_envs.clone();
        merged_envs.extend(task_envs.iter().map(|(k, v)| (k.clone(), v.clone())));

        // Step 2: Evaluate the merged environment (task envs can reference global envs)
        let evaluated_merged = if merged_envs.is_empty() {
            HashMap::new()
        } else {
            env_eval::evaluate_envs(&merged_envs, Some(working_dir))?
        };

        Ok(evaluated_merged)
    }

    /// Resolve file glob patterns to canonical file paths
    fn resolve_file_globs(patterns: &[String], cwd: &std::path::Path) -> Vec<String> {
        let mut resolved_files = Vec::new();

        for pattern in patterns {
            // Convert pattern to absolute path using provided cwd
            let pattern_path = if std::path::Path::new(pattern).is_absolute() {
                pattern.clone()
            } else {
                cwd.join(pattern).to_string_lossy().to_string()
            };

            // Use glob to expand patterns
            match glob::glob(&pattern_path) {
                Ok(paths) => {
                    let mut found_files = false;
                    for path in paths.flatten() {
                        found_files = true;
                        if let Ok(canonical) = path.canonicalize() {
                            resolved_files.push(canonical.to_string_lossy().to_string());
                        } else {
                            // If canonicalize fails, still add the path as-is
                            resolved_files.push(path.to_string_lossy().to_string());
                        }
                    }

                    // If glob succeeded but found no files, convert to absolute path anyway
                    if !found_files {
                        let abs_path = if std::path::Path::new(pattern).is_absolute() {
                            pattern.clone()
                        } else {
                            cwd.join(pattern).to_string_lossy().to_string()
                        };
                        resolved_files.push(abs_path);
                    }
                }
                Err(_) => {
                    // If glob fails, convert to absolute path anyway
                    let abs_path = if std::path::Path::new(pattern).is_absolute() {
                        pattern.clone()
                    } else {
                        cwd.join(pattern).to_string_lossy().to_string()
                    };
                    resolved_files.push(abs_path);
                }
            }
        }

        resolved_files
    }
}

fn calculate_hash(action: &String) -> String {
    let mut hasher = Sha256::new();
    hasher.update(action.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)[..8].to_string()
}
