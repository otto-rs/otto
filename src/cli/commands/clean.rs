use chrono::{DateTime, Utc};
use eyre::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::executor::state::RunMetadata;

pub struct CleanCommand {
    keep_days: u64,
    dry_run: bool,
    project_filter: Option<String>,
}

struct RunInfo {
    path: PathBuf,
    project_hash: String,
    timestamp: u64,
    age_days: u64,
    size_bytes: u64,
    ottofile_path: Option<PathBuf>,
}

impl CleanCommand {
    pub fn new(keep_days: u64, dry_run: bool, project_filter: Option<String>) -> Self {
        Self {
            keep_days,
            dry_run,
            project_filter,
        }
    }

    pub async fn execute(&self) -> Result<()> {
        let otto_home = self.get_otto_home()?;

        if !otto_home.exists() {
            println!("No ~/.otto directory found");
            return Ok(());
        }

        println!("Scanning {} for old runs...", otto_home.display());

        let mut runs_to_delete = self.scan_for_old_runs(&otto_home)?;

        if runs_to_delete.is_empty() {
            println!("No runs older than {} days found", self.keep_days);
            return Ok(());
        }

        // Sort by timestamp (oldest first)
        runs_to_delete.sort_by_key(|r| r.timestamp);

        let total_size = runs_to_delete.iter().map(|r| r.size_bytes).sum::<u64>();

        println!(
            "\nFound {} runs older than {} days ({} total)",
            runs_to_delete.len(),
            self.keep_days,
            self.format_size(total_size)
        );

        if self.dry_run {
            println!("\nDry run - showing what would be deleted:\n");
            for run in &runs_to_delete {
                let date_time = self.format_timestamp(run.timestamp);
                let ottofile_display = run
                    .ottofile_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<unknown>".to_string());
                println!(
                    "  [{}] {} - {} ({} days old, {})",
                    run.project_hash,
                    date_time,
                    ottofile_display,
                    run.age_days,
                    self.format_size(run.size_bytes)
                );
            }
            println!("\nRun without --dry-run to actually delete these runs");
        } else {
            println!("\nDeleting runs...\n");
            let mut deleted_size = 0u64;

            for run in &runs_to_delete {
                match fs::remove_dir_all(&run.path) {
                    Ok(()) => {
                        deleted_size += run.size_bytes;
                        let date_time = self.format_timestamp(run.timestamp);
                        let ottofile_display = run
                            .ottofile_path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "<unknown>".to_string());
                        println!(
                            "  Deleted [{}] {} - {} ({})",
                            run.project_hash,
                            date_time,
                            ottofile_display,
                            self.format_size(run.size_bytes)
                        );
                    }
                    Err(e) => {
                        eprintln!("  Failed to delete {}: {}", run.path.display(), e);
                    }
                }
            }

            println!("\nFreed {} of disk space", self.format_size(deleted_size));
        }

        Ok(())
    }

    fn scan_for_old_runs(&self, otto_home: &Path) -> Result<Vec<RunInfo>> {
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();

        let mut runs_to_delete = Vec::new();

        for entry in fs::read_dir(otto_home)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name,
                None => continue,
            };

            // Skip otto.db and other non-project directories
            if dir_name == "otto.db" || !dir_name.starts_with("otto-") {
                continue;
            }

            // Apply project filter if specified
            if let Some(ref filter) = self.project_filter
                && !dir_name.contains(filter)
            {
                continue;
            }

            // Extract project hash from directory name (e.g., "otto-6b20a2e4" -> "6b20a2e4")
            let project_hash = dir_name.strip_prefix("otto-").unwrap_or("unknown").to_string();

            // Scan timestamp directories within this project
            runs_to_delete.extend(self.scan_project_runs(&path, &project_hash, now)?);
        }

        Ok(runs_to_delete)
    }

    fn scan_project_runs(&self, project_dir: &Path, project_hash: &str, now: u64) -> Result<Vec<RunInfo>> {
        let mut runs = Vec::new();

        for entry in fs::read_dir(project_dir)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name,
                None => continue,
            };

            // Skip .cache directory
            if dir_name == ".cache" {
                continue;
            }

            // Try to parse as timestamp
            if let Ok(timestamp) = dir_name.parse::<u64>() {
                let age_seconds = now.saturating_sub(timestamp);
                let age_days = age_seconds / 86400;

                if age_days > self.keep_days {
                    let size_bytes = Self::calculate_dir_size(&path)?;

                    // Try to read ottofile path from run.yaml
                    let ottofile_path = self.read_ottofile_path(&path);

                    runs.push(RunInfo {
                        path,
                        project_hash: project_hash.to_string(),
                        timestamp,
                        age_days,
                        size_bytes,
                        ottofile_path,
                    });
                }
            }
        }

        Ok(runs)
    }

    fn read_ottofile_path(&self, run_dir: &Path) -> Option<PathBuf> {
        let run_yaml_path = run_dir.join("run.yaml");
        if !run_yaml_path.exists() {
            return None;
        }

        let content = fs::read_to_string(&run_yaml_path).ok()?;
        let metadata: RunMetadata = serde_yaml::from_str(&content).ok()?;
        metadata.ottofile
    }

    fn format_timestamp(&self, timestamp: u64) -> String {
        let dt = DateTime::from_timestamp(timestamp as i64, 0).unwrap_or(DateTime::<Utc>::MIN_UTC);
        dt.format("%Y-%m-%d %H:%M:%S").to_string()
    }

    fn format_size(&self, bytes: u64) -> String {
        if bytes < 1024 {
            format!("{bytes} B")
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else if bytes < 1024 * 1024 * 1024 {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        } else {
            format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
        }
    }

    fn calculate_dir_size(path: &Path) -> Result<u64> {
        let mut total_size = 0u64;

        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let entry_path = entry.path();

                if entry_path.is_dir() {
                    total_size += Self::calculate_dir_size(&entry_path)?;
                } else {
                    total_size += entry.metadata()?.len();
                }
            }
        }

        Ok(total_size)
    }

    fn get_otto_home(&self) -> Result<PathBuf> {
        let home = std::env::var("HOME").context("Failed to get HOME environment variable")?;
        Ok(PathBuf::from(home).join(".otto"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_run(base_dir: &Path, project: &str, timestamp: u64, size_kb: usize) -> Result<()> {
        let run_dir = base_dir
            .join(format!("otto-{}", project))
            .join(timestamp.to_string())
            .join("tasks");
        fs::create_dir_all(&run_dir)?;

        // Create a file with specified size
        let file_path = run_dir.join("test.log");
        let content = vec![0u8; size_kb * 1024];
        fs::write(file_path, content)?;

        Ok(())
    }

    #[tokio::test]
    async fn test_scan_empty_directory() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let cmd = CleanCommand::new(30, true, None);

        let runs = cmd.scan_for_old_runs(temp_dir.path())?;
        assert_eq!(runs.len(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn test_scan_with_old_runs() -> Result<()> {
        let temp_dir = TempDir::new()?;

        // Create runs with different ages
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
        let old_timestamp = now - (40 * 86400); // 40 days old
        let recent_timestamp = now - (10 * 86400); // 10 days old

        create_test_run(temp_dir.path(), "abc123", old_timestamp, 100)?;
        create_test_run(temp_dir.path(), "abc123", recent_timestamp, 50)?;

        let cmd = CleanCommand::new(30, true, None);
        let runs = cmd.scan_for_old_runs(temp_dir.path())?;

        // Should only find the 40-day-old run
        assert_eq!(runs.len(), 1);
        assert!(runs[0].age_days >= 39 && runs[0].age_days <= 41);
        Ok(())
    }

    #[tokio::test]
    async fn test_calculate_dir_size() -> Result<()> {
        let temp_dir = TempDir::new()?;
        create_test_run(temp_dir.path(), "test", 1234567890, 100)?;

        let run_dir = temp_dir.path().join("otto-test").join("1234567890");
        let size = CleanCommand::calculate_dir_size(&run_dir)?;

        // Should be approximately 100KB (may vary slightly due to filesystem overhead)
        assert!(size >= 100 * 1024);
        assert!(size < 110 * 1024);
        Ok(())
    }

    #[tokio::test]
    async fn test_project_filter() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
        let old_timestamp = now - (40 * 86400);

        create_test_run(temp_dir.path(), "abc123", old_timestamp, 100)?;
        create_test_run(temp_dir.path(), "def456", old_timestamp, 100)?;

        let cmd = CleanCommand::new(30, true, Some("abc123".to_string()));
        let runs = cmd.scan_for_old_runs(temp_dir.path())?;

        // Should only find runs from abc123 project
        assert_eq!(runs.len(), 1);
        assert!(runs[0].path.to_string_lossy().contains("abc123"));
        Ok(())
    }

    #[tokio::test]
    async fn test_read_ottofile_path_with_metadata() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let run_dir = temp_dir.path().join("test_run");
        fs::create_dir_all(&run_dir)?;

        // Create run.yaml with ottofile path
        let metadata = RunMetadata::minimal(
            Some(PathBuf::from("/path/to/otto.yml")),
            "abc123".to_string(),
            1234567890,
        );
        let yaml_content = serde_yaml::to_string(&metadata)?;
        fs::write(run_dir.join("run.yaml"), yaml_content)?;

        let cmd = CleanCommand::new(30, true, None);
        let ottofile_path = cmd.read_ottofile_path(&run_dir);

        assert_eq!(ottofile_path, Some(PathBuf::from("/path/to/otto.yml")));
        Ok(())
    }

    #[tokio::test]
    async fn test_read_ottofile_path_missing_file() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let run_dir = temp_dir.path().join("test_run");
        fs::create_dir_all(&run_dir)?;

        let cmd = CleanCommand::new(30, true, None);
        let ottofile_path = cmd.read_ottofile_path(&run_dir);

        assert_eq!(ottofile_path, None);
        Ok(())
    }

    #[tokio::test]
    async fn test_read_ottofile_path_no_ottofile_field() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let run_dir = temp_dir.path().join("test_run");
        fs::create_dir_all(&run_dir)?;

        // Create run.yaml without ottofile field
        let metadata = RunMetadata::minimal(None, "abc123".to_string(), 1234567890);
        let yaml_content = serde_yaml::to_string(&metadata)?;
        fs::write(run_dir.join("run.yaml"), yaml_content)?;

        let cmd = CleanCommand::new(30, true, None);
        let ottofile_path = cmd.read_ottofile_path(&run_dir);

        assert_eq!(ottofile_path, None);
        Ok(())
    }

    #[tokio::test]
    async fn test_read_ottofile_path_malformed_yaml() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let run_dir = temp_dir.path().join("test_run");
        fs::create_dir_all(&run_dir)?;

        // Create malformed YAML
        fs::write(run_dir.join("run.yaml"), "invalid: yaml: content: {")?;

        let cmd = CleanCommand::new(30, true, None);
        let ottofile_path = cmd.read_ottofile_path(&run_dir);

        assert_eq!(ottofile_path, None);
        Ok(())
    }

    #[tokio::test]
    async fn test_format_timestamp() -> Result<()> {
        let cmd = CleanCommand::new(30, true, None);

        // Test a known timestamp
        let timestamp = 1609459200; // 2021-01-01 00:00:00 UTC
        let formatted = cmd.format_timestamp(timestamp);

        assert_eq!(formatted, "2021-01-01 00:00:00");
        Ok(())
    }

    #[tokio::test]
    async fn test_format_size_bytes() -> Result<()> {
        let cmd = CleanCommand::new(30, true, None);

        assert_eq!(cmd.format_size(0), "0 B");
        assert_eq!(cmd.format_size(512), "512 B");
        assert_eq!(cmd.format_size(1023), "1023 B");
        Ok(())
    }

    #[tokio::test]
    async fn test_format_size_kilobytes() -> Result<()> {
        let cmd = CleanCommand::new(30, true, None);

        assert_eq!(cmd.format_size(1024), "1.0 KB");
        assert_eq!(cmd.format_size(2048), "2.0 KB");
        assert_eq!(cmd.format_size(1536), "1.5 KB");
        assert_eq!(cmd.format_size(100 * 1024), "100.0 KB");
        Ok(())
    }

    #[tokio::test]
    async fn test_format_size_megabytes() -> Result<()> {
        let cmd = CleanCommand::new(30, true, None);

        assert_eq!(cmd.format_size(1024 * 1024), "1.0 MB");
        assert_eq!(cmd.format_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(cmd.format_size(1536 * 1024), "1.5 MB");
        assert_eq!(cmd.format_size(100 * 1024 * 1024), "100.0 MB");
        Ok(())
    }

    #[tokio::test]
    async fn test_format_size_gigabytes() -> Result<()> {
        let cmd = CleanCommand::new(30, true, None);

        assert_eq!(cmd.format_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(cmd.format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
        assert_eq!(cmd.format_size(1536 * 1024 * 1024), "1.5 GB");
        Ok(())
    }

    #[tokio::test]
    async fn test_runs_sorted_by_timestamp() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();

        // Create runs with different timestamps in random order
        let timestamp1 = now - (60 * 86400); // 60 days old
        let timestamp2 = now - (45 * 86400); // 45 days old
        let timestamp3 = now - (50 * 86400); // 50 days old

        create_test_run(temp_dir.path(), "abc123", timestamp2, 100)?;
        create_test_run(temp_dir.path(), "abc123", timestamp1, 100)?;
        create_test_run(temp_dir.path(), "abc123", timestamp3, 100)?;

        let cmd = CleanCommand::new(30, true, None);
        let mut runs = cmd.scan_for_old_runs(temp_dir.path())?;

        // Sort by timestamp
        runs.sort_by_key(|r| r.timestamp);

        // Should be sorted oldest to newest
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].timestamp, timestamp1);
        assert_eq!(runs[1].timestamp, timestamp3);
        assert_eq!(runs[2].timestamp, timestamp2);
        Ok(())
    }

    #[tokio::test]
    async fn test_scan_with_ottofile_metadata() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
        let old_timestamp = now - (40 * 86400);

        // Create run with metadata
        let project_dir = temp_dir.path().join("otto-abc123");
        let run_dir = project_dir.join(old_timestamp.to_string());
        let tasks_dir = run_dir.join("tasks");
        fs::create_dir_all(&tasks_dir)?;

        // Create test file
        let file_path = tasks_dir.join("test.log");
        let content = vec![0u8; 100 * 1024];
        fs::write(file_path, content)?;

        // Create run.yaml with ottofile path
        let metadata = RunMetadata::minimal(
            Some(PathBuf::from("/test/project/otto.yml")),
            "abc123".to_string(),
            old_timestamp,
        );
        let yaml_content = serde_yaml::to_string(&metadata)?;
        fs::write(run_dir.join("run.yaml"), yaml_content)?;

        let cmd = CleanCommand::new(30, true, None);
        let runs = cmd.scan_for_old_runs(temp_dir.path())?;

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].project_hash, "abc123");
        assert_eq!(runs[0].timestamp, old_timestamp);
        assert_eq!(runs[0].ottofile_path, Some(PathBuf::from("/test/project/otto.yml")));
        assert!(runs[0].size_bytes >= 100 * 1024);
        Ok(())
    }

    #[tokio::test]
    async fn test_scan_without_ottofile_metadata() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
        let old_timestamp = now - (40 * 86400);

        // Create run without metadata (old run)
        create_test_run(temp_dir.path(), "def456", old_timestamp, 100)?;

        let cmd = CleanCommand::new(30, true, None);
        let runs = cmd.scan_for_old_runs(temp_dir.path())?;

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].project_hash, "def456");
        assert_eq!(runs[0].ottofile_path, None);
        Ok(())
    }
}
