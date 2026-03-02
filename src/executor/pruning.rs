use crate::cfg::otto::RetentionSpec;
use crate::cli::CleanCommand;
use eyre::Result;
use log::warn;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Resolve the otto home directory.
///
/// Uses `$OTTO_HOME` if set, otherwise `$HOME/.otto`.
pub fn resolve_otto_home() -> Result<PathBuf> {
    if let Ok(otto_home) = std::env::var("OTTO_HOME") {
        Ok(PathBuf::from(otto_home))
    } else {
        let home = std::env::var("HOME").map_err(|e| eyre::eyre!("Failed to get HOME: {}", e))?;
        Ok(PathBuf::from(home).join(".otto"))
    }
}

/// Run automatic pruning if enough time has elapsed since the last prune.
///
/// This is best-effort: errors are logged but not propagated.
/// Called after task execution completes (even on failure).
pub async fn auto_prune(otto_home: &Path, retention: &RetentionSpec) {
    if !retention.auto_prune {
        return;
    }

    let marker = otto_home.join(".last_prune");
    if let Ok(meta) = fs::metadata(&marker)
        && let Ok(modified) = meta.modified()
        && let Ok(age) = modified.elapsed()
        && age < Duration::from_secs(retention.prune_interval_hours * 3600)
    {
        return; // Fast path: too soon
    }
    // .last_prune missing or stale → prune now

    log::info!("Auto-pruning old runs (interval: {}h)", retention.prune_interval_hours);

    let cmd = CleanCommand {
        keep_days: retention.keep_days,
        keep_last: Some(retention.keep_last),
        keep_failed: Some(retention.keep_failed),
        dry_run: false,
        project_filter: None,
        no_db: false,
        quiet: true,
    };

    if let Err(e) = cmd.execute().await {
        warn!("Auto-prune failed: {}", e);
        return;
    }

    // Touch marker file
    if let Err(e) = fs::File::create(&marker) {
        warn!("Failed to update .last_prune marker: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_resolve_otto_home_default() {
        // When OTTO_HOME is not set, should use $HOME/.otto
        // This test just verifies it doesn't panic
        let home = resolve_otto_home();
        assert!(home.is_ok());
    }

    #[test]
    fn test_resolve_otto_home_with_env() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            std::env::set_var("OTTO_HOME", temp_dir.path());
        }
        let home = resolve_otto_home().unwrap();
        assert_eq!(home, temp_dir.path());
        unsafe {
            std::env::remove_var("OTTO_HOME");
        }
    }

    #[tokio::test]
    async fn test_auto_prune_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let retention = RetentionSpec {
            auto_prune: false,
            ..Default::default()
        };
        // Should return immediately without error
        auto_prune(temp_dir.path(), &retention).await;
        // .last_prune should not be created
        assert!(!temp_dir.path().join(".last_prune").exists());
    }

    #[tokio::test]
    async fn test_auto_prune_throttle_skip() {
        let temp_dir = TempDir::new().unwrap();
        let marker = temp_dir.path().join(".last_prune");
        // Create a fresh marker (just now)
        fs::File::create(&marker).unwrap();

        let retention = RetentionSpec {
            auto_prune: true,
            prune_interval_hours: 24,
            ..Default::default()
        };

        // Should skip because marker is fresh
        auto_prune(temp_dir.path(), &retention).await;
        // Marker should still exist but not be re-touched significantly
    }

    #[tokio::test]
    async fn test_auto_prune_runs_when_stale() {
        let temp_dir = TempDir::new().unwrap();
        let marker = temp_dir.path().join(".last_prune");

        // Create a marker with old mtime
        fs::File::create(&marker).unwrap();
        let old_time = std::time::SystemTime::now() - Duration::from_secs(25 * 3600);
        filetime::set_file_mtime(&marker, filetime::FileTime::from_system_time(old_time)).unwrap();

        let retention = RetentionSpec {
            auto_prune: true,
            prune_interval_hours: 24,
            ..Default::default()
        };

        // Should prune and update marker
        auto_prune(temp_dir.path(), &retention).await;
        // Marker should be updated to recent time
        let meta = fs::metadata(&marker).unwrap();
        let age = meta.modified().unwrap().elapsed().unwrap();
        assert!(age < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_auto_prune_creates_marker_when_missing() {
        let temp_dir = TempDir::new().unwrap();
        assert!(!temp_dir.path().join(".last_prune").exists());

        let retention = RetentionSpec {
            auto_prune: true,
            prune_interval_hours: 24,
            ..Default::default()
        };

        auto_prune(temp_dir.path(), &retention).await;
        assert!(temp_dir.path().join(".last_prune").exists());
    }
}
