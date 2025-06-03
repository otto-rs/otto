use std::path::{Path, PathBuf};
use std::time::SystemTime;
use eyre::{eyre, Result};
use sha2::{Sha256, Digest};
use expanduser::expanduser;

/// Handles Otto's directory structure and storage paths
pub struct Workspace {
    // Base paths
    home: PathBuf,          // ~/.otto
    root: PathBuf,          // Current project directory
    hash: String,          // First 8 chars of project path hash
    time: u64,             // Current run timestamp

    // Computed paths
    project: PathBuf,      // <name>-<hash>
    cache: PathBuf,        // <name>-<hash>/.cache
    run: PathBuf,          // <name>-<hash>/<timestamp>
}

impl Workspace {
    /// Create new workspace for a project
    pub async fn new(root: PathBuf) -> Result<Self> {
        // Expand any ~ in the root path first
        let root = expanduser(root.to_string_lossy().to_string())?;

        // Get canonical project root, creating parent dirs if needed
        let root = if !root.exists() {
            if let Some(parent) = root.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            root
        } else {
            root.canonicalize()
                .map_err(|e| eyre!("Failed to canonicalize project root: {}", e))?
        };

        // Get project name from last component (unused but kept for future use)
        let _name = root.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| eyre!("Invalid project root path"))?;

        // Calculate project path hash - use only first 8 chars
        let mut hasher = Sha256::new();
        hasher.update(root.to_string_lossy().as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        let hash = hash[..8].to_string();

        // Get timestamp for this run
        let time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();

        // Get absolute path to ~/.otto by expanding HOME
        let home_dir = std::env::var("HOME")
            .map_err(|e| eyre!("Failed to get HOME directory: {}", e))?;
        let home = PathBuf::from(home_dir).join(".otto");

        // Build computed paths - simple structure with otto-<hash>
        let project = home.join(format!("otto-{}", hash));
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
        self.task(task).join(format!("script.{}", ext))
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
        self.run.join(format!("{}.yaml", name))
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
            .map_err(|e| eyre!("Path {} is not relative to root {}: {}", 
                path.as_ref().display(), 
                self.root.display(), 
                e))
    }

    /// Check if a path is within the project root
    pub fn is_in_project<P: AsRef<Path>>(&self, path: P) -> bool {
        path.as_ref().starts_with(&self.root)
    }

    /// Get a path relative to the project root
    pub fn join_root<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        self.root.join(path)
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