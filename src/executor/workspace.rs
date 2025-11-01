use expanduser::expanduser;
use eyre::{Result, eyre};
use serde::{Deserialize, Serialize};
use serde_yaml;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub prog: String,
    pub cwd: PathBuf,
    pub user: String,
    pub timestamp: u64,
    pub hash: String,
    pub ottofile: Option<PathBuf>,
    pub args: Vec<String>,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionContext {
    /// Create a minimal execution context for testing
    pub fn new() -> Self {
        Self {
            prog: "otto".to_string(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            user: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            hash: "test".to_string(),
            ottofile: None,
            args: vec!["otto".to_string()],
        }
    }
}

/// Handles Otto's directory structure and storage paths
pub struct Workspace {
    // Base paths
    home: PathBuf, // ~/.otto
    root: PathBuf, // Current project directory
    hash: String,  // First 8 chars of project path hash
    time: u64,     // Current run timestamp

    // Computed paths
    project: PathBuf, // <name>-<hash>
    cache: PathBuf,   // <name>-<hash>/.cache
    run: PathBuf,     // <name>-<hash>/<timestamp>
}

impl Workspace {
    /// Create new workspace for a project
    pub async fn new(root: PathBuf) -> Result<Self> {
        // Expand any ~ in the root path first
        let root = expanduser(root.to_string_lossy())?;

        // Get canonical project root, creating parent dirs if needed
        let root = if !root.exists() {
            if let Some(parent) = root.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            // For non-existent paths, still try to canonicalize the parent and join the last component
            if let Some(parent) = root.parent() {
                let canonical_parent = parent
                    .canonicalize()
                    .map_err(|e| eyre!("Failed to canonicalize parent directory: {}", e))?;
                if let Some(file_name) = root.file_name() {
                    canonical_parent.join(file_name)
                } else {
                    root
                }
            } else {
                root
            }
        } else {
            root.canonicalize()
                .map_err(|e| eyre!("Failed to canonicalize project root: {}", e))?
        };

        // Get project name from last component (unused but kept for future use)
        let _name = root
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                // Fallback for cases where file_name() returns None (like root directories)
                "otto_project".to_string()
            });

        // Calculate project path hash - use only first 8 chars
        let mut hasher = Sha256::new();
        hasher.update(root.to_string_lossy().as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        let hash = hash[..8].to_string();

        Self::new_with_hash(root, hash).await
    }

    /// Create new workspace with a specific hash
    pub async fn new_with_hash(root: PathBuf, hash: String) -> Result<Self> {
        // Get timestamp for this run
        let time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();

        // Get absolute path to ~/.otto by expanding HOME
        let home_dir = std::env::var("HOME").map_err(|e| eyre!("Failed to get HOME directory: {}", e))?;
        let home = PathBuf::from(home_dir).join(".otto");

        // Build computed paths - simple structure with otto-<hash>
        let project = home.join(format!("otto-{hash}"));
        let cache = project.join(".cache");
        let run = project.join(time.to_string());

        Ok(Self {
            home,
            root,
            hash,
            time,
            project,
            cache,
            run,
        })
    }

    /// Initialize workspace directories
    pub async fn init(&self) -> Result<()> {
        // Create all required directories
        for path in [&self.home, &self.project, &self.cache, &self.run] {
            tokio::fs::create_dir_all(path)
                .await
                .map_err(|e| eyre!("Failed to create directory {}: {}", path.display(), e))?;
        }

        // Create tasks directory
        tokio::fs::create_dir_all(self.run.join("tasks"))
            .await
            .map_err(|e| eyre!("Failed to create tasks directory: {}", e))?;

        Ok(())
    }

    /// Get path for a cached script
    pub fn script_cache(&self, task: &str, hash: &str) -> PathBuf {
        self.cache.join(task).join(hash)
    }

    /// Get task directory for current run
    pub fn task(&self, name: &str) -> PathBuf {
        self.run.join("tasks").join(name)
    }

    /// Get path for task script symlink
    pub fn script(&self, task: &str, is_python: bool) -> PathBuf {
        let ext = if is_python { "py" } else { "sh" };
        self.task(task).join(format!("script.{ext}"))
    }

    /// Get path for task output file
    pub fn output(&self, task: &str) -> PathBuf {
        self.task(task).join("output.json")
    }

    /// Get path for task stdout log
    pub fn stdout(&self, task: &str) -> PathBuf {
        self.task(task).join("stdout.log")
    }

    /// Get path for task stderr log
    pub fn stderr(&self, task: &str) -> PathBuf {
        self.task(task).join("stderr.log")
    }

    /// Get path for task artifacts directory
    pub fn artifacts(&self, task: &str) -> PathBuf {
        self.task(task).join("artifacts")
    }

    /// Get path for run metadata files
    pub fn metadata(&self, name: &str) -> PathBuf {
        self.run.join(format!("{name}.yaml"))
    }

    /// Verify a task's directory structure exists
    pub async fn verify_task(&self, name: &str) -> Result<()> {
        let task_dir = self.task(name);

        // Check if task directory exists
        if !task_dir.exists() {
            return Err(eyre!("Task directory does not exist: {}", task_dir.display()));
        }

        // Verify script symlink
        let script = task_dir.join("script.*");
        if !script.exists() {
            return Err(eyre!("Task script not found: {}", script.display()));
        }

        // Verify it's a valid symlink
        let script_target = tokio::fs::read_link(&script)
            .await
            .map_err(|e| eyre!("Failed to read script symlink: {}", e))?;

        if !script_target.exists() {
            return Err(eyre!("Script target does not exist: {}", script_target.display()));
        }

        Ok(())
    }

    /// Get the project root directory
    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// Get the run directory for this execution
    pub fn run(&self) -> &PathBuf {
        &self.run
    }

    /// Get the unique hash for this project
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Get the timestamp for this run
    pub fn timestamp(&self) -> u64 {
        self.time
    }

    /// Get the relative path from project root to a file
    pub fn relative_to_root<P: AsRef<Path>>(&self, path: P) -> Result<PathBuf> {
        path.as_ref()
            .strip_prefix(&self.root)
            .map(|p| p.to_path_buf())
            .map_err(|e| {
                eyre!(
                    "Path {} is not relative to root {}: {}",
                    path.as_ref().display(),
                    self.root.display(),
                    e
                )
            })
    }

    /// Check if a path is within the project root
    pub fn is_in_project<P: AsRef<Path>>(&self, path: P) -> bool {
        path.as_ref().starts_with(&self.root)
    }

    /// Get a path relative to the project root
    pub fn join_root<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        self.root.join(path)
    }

    /// Save execution context metadata to run.yaml
    pub async fn save_execution_context(&self, context: ExecutionContext) -> Result<()> {
        let run_yaml_path = self.metadata("run");
        let yaml_content =
            serde_yaml::to_string(&context).map_err(|e| eyre!("Failed to serialize execution context: {}", e))?;

        tokio::fs::write(&run_yaml_path, yaml_content)
            .await
            .map_err(|e| eyre!("Failed to write run.yaml: {}", e))?;

        Ok(())
    }

    /// Save task-specific execution context to task directory
    pub async fn save_task_context(&self, task_name: &str, context: &ExecutionContext) -> Result<()> {
        let task_run_yaml = self.task(task_name).join("run.yaml");
        let yaml_content =
            serde_yaml::to_string(context).map_err(|e| eyre!("Failed to serialize task context: {}", e))?;

        tokio::fs::write(&task_run_yaml, yaml_content)
            .await
            .map_err(|e| eyre!("Failed to write task run.yaml: {}", e))?;

        Ok(())
    }

    // === NEW METHODS FOR ACTION PROCESSING ===

    /// Get task directory path (alias for existing task() method)
    pub fn task_dir(&self, task_name: &str) -> PathBuf {
        self.task(task_name)
    }

    /// Get task input directory path
    pub fn task_input_dir(&self, task_name: &str) -> PathBuf {
        self.task(task_name).join("inputs")
    }

    /// Get task output directory path
    pub fn task_output_dir(&self, task_name: &str) -> PathBuf {
        self.task(task_name).join("outputs")
    }

    /// Get task output file path
    pub fn task_output_file(&self, task_name: &str) -> PathBuf {
        self.task(task_name).join(format!("output.{task_name}.json"))
    }

    /// Get task input file path for a specific dependency
    pub fn task_input_file(&self, task_name: &str, dep_name: &str) -> PathBuf {
        self.task(task_name).join(format!("input.{dep_name}.json"))
    }

    /// Get task script file path with extension
    pub fn task_script_file(&self, task_name: &str, extension: &str) -> PathBuf {
        self.task(task_name).join(format!("script.{extension}"))
    }

    /// Get the current run directory
    pub fn current_run_dir(&self) -> &PathBuf {
        &self.run
    }

    /// Get the project root directory (alias for root())
    pub fn project_root(&self) -> &PathBuf {
        &self.root
    }

    /// Get path for bash builtin functions
    pub fn bash_builtins(&self) -> PathBuf {
        self.project.join("builtins.sh")
    }

    /// Get path for python builtin functions
    pub fn python_builtins(&self) -> PathBuf {
        self.project.join("builtins.py")
    }

    /// Get the cache directory for this workspace
    pub fn cache_dir(&self) -> &PathBuf {
        &self.cache
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_workspace_creation() -> Result<()> {
        let temp = TempDir::new()?;
        let root = temp.path().to_path_buf();

        let ws = Workspace::new(root.clone()).await?;
        ws.init().await?;

        assert!(ws.home.exists());
        assert!(ws.project.exists());
        assert!(ws.cache.exists());
        assert!(ws.run.exists());
        assert!(ws.run.join("tasks").exists());

        Ok(())
    }

    #[tokio::test]
    async fn test_task_paths() -> Result<()> {
        let temp = TempDir::new()?;
        let root = temp.path().to_path_buf();

        let ws = Workspace::new(root.clone()).await?;
        ws.init().await?;

        let task = "test_task";
        let script_hash = "abcd1234";

        assert!(ws.script_cache(task, script_hash).starts_with(&ws.cache));
        assert!(ws.task(task).starts_with(&ws.run));
        assert!(ws.script(task, false).ends_with("script.sh"));
        assert!(ws.script(task, true).ends_with("script.py"));
        assert!(ws.output(task).ends_with("output.json"));
        assert!(ws.stdout(task).ends_with("stdout.log"));
        assert!(ws.stderr(task).ends_with("stderr.log"));
        assert!(ws.artifacts(task).ends_with("artifacts"));

        Ok(())
    }

    #[tokio::test]
    async fn test_metadata_paths() -> Result<()> {
        let temp = TempDir::new()?;
        let root = temp.path().to_path_buf();

        let ws = Workspace::new(root.clone()).await?;

        assert!(ws.metadata("run").ends_with("run.yaml"));
        assert!(ws.metadata("env").ends_with("env.yaml"));
        assert!(ws.metadata("cmdline").ends_with("cmdline.yaml"));

        Ok(())
    }
}
