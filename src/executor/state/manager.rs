use eyre::{Context, Result};
use rusqlite::{OptionalExtension, params};
use std::path::PathBuf;
use std::time::SystemTime;

use super::db::DatabaseManager;
use super::metadata::RunMetadata;
use super::schema::RunStatus;

/// State manager for recording and querying run/task state
pub struct StateManager {
    db: DatabaseManager,
}

/// A run record from the database
#[derive(Debug, Clone)]
pub struct RunRecord {
    pub id: i64,
    pub project_id: i64,
    pub timestamp: u64,
    pub status: RunStatus,
    pub duration_seconds: Option<f64>,
    pub size_bytes: Option<u64>,
    pub ottofile_path: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub user: Option<String>,
    pub hostname: Option<String>,
    pub args: Option<Vec<String>>,
    pub ended_at: Option<u64>,
}

impl StateManager {
    /// Create a new StateManager with the default database location
    pub fn new() -> Result<Self> {
        let db = DatabaseManager::open_default()?;
        Ok(Self { db })
    }

    /// Create a StateManager with a custom database path
    pub fn with_db_path(db_path: PathBuf) -> Result<Self> {
        let db = DatabaseManager::new(db_path)?;
        Ok(Self { db })
    }

    /// Try to create a StateManager, returning None if DB is unavailable
    /// This enables graceful degradation
    pub fn try_new() -> Option<Self> {
        Self::new().ok()
    }

    /// Record the start of a run
    /// Returns the database run ID if successful
    pub fn record_run_start(&self, metadata: &RunMetadata) -> Result<i64> {
        self.db.with_connection(|conn| {
            // First, ensure project exists
            let project_id = self.ensure_project(conn, &metadata.hash, metadata.ottofile.as_ref())?;

            // Serialize args if present
            let args_json = metadata
                .args
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .context("Failed to serialize args")?;

            // Insert run record
            conn.execute(
                "INSERT INTO runs (
                    project_id, timestamp, status, ottofile_path, cwd, user, hostname, args
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    project_id,
                    metadata.timestamp as i64,
                    RunStatus::Running.as_str(),
                    metadata.ottofile.as_ref().map(|p| p.to_string_lossy().to_string()),
                    metadata.cwd.as_ref().map(|p| p.to_string_lossy().to_string()),
                    metadata.user,
                    metadata.hostname,
                    args_json,
                ],
            )?;

            let run_id = conn.last_insert_rowid();

            // Update project last_seen and run_count
            conn.execute(
                "UPDATE projects
                 SET last_seen = ?1, run_count = run_count + 1
                 WHERE id = ?2",
                params![metadata.timestamp as i64, project_id],
            )?;

            Ok(run_id)
        })
    }

    /// Record the completion of a run
    pub fn record_run_complete(&self, timestamp: u64, status: RunStatus, size_bytes: Option<u64>) -> Result<()> {
        let ended_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .context("Failed to get current time")?
            .as_secs();

        let duration_seconds = ended_at.saturating_sub(timestamp) as f64;

        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE runs
                 SET status = ?1, duration_seconds = ?2, size_bytes = ?3, ended_at = ?4
                 WHERE timestamp = ?5",
                params![
                    status.as_str(),
                    duration_seconds,
                    size_bytes.map(|s| s as i64),
                    ended_at as i64,
                    timestamp as i64,
                ],
            )?;
            Ok(())
        })
    }

    /// Get recent runs, optionally filtered by project hash
    pub fn get_recent_runs(&self, limit: usize, project_filter: Option<&str>) -> Result<Vec<RunRecord>> {
        self.db.with_connection(|conn| {
            let query = if let Some(_project_hash) = project_filter {
                "SELECT r.id, r.project_id, r.timestamp, r.status, r.duration_seconds,
                            r.size_bytes, r.ottofile_path, r.cwd, r.user, r.hostname, r.args, r.ended_at
                     FROM runs r
                     JOIN projects p ON r.project_id = p.id
                     WHERE p.hash = ?1
                     ORDER BY r.timestamp DESC
                     LIMIT ?2"
            } else {
                "SELECT r.id, r.project_id, r.timestamp, r.status, r.duration_seconds,
                            r.size_bytes, r.ottofile_path, r.cwd, r.user, r.hostname, r.args, r.ended_at
                     FROM runs r
                     ORDER BY r.timestamp DESC
                     LIMIT ?1"
            };

            let mut stmt = conn.prepare(query)?;

            let rows = if let Some(project_hash) = project_filter {
                stmt.query_map(params![project_hash, limit as i64], Self::row_to_run_record)?
            } else {
                stmt.query_map(params![limit as i64], Self::row_to_run_record)?
            };

            rows.collect::<Result<Vec<_>, _>>().context("Failed to fetch runs")
        })
    }

    /// Helper to convert a database row to a RunRecord
    fn row_to_run_record(row: &rusqlite::Row) -> rusqlite::Result<RunRecord> {
        let args_json: Option<String> = row.get(10)?;
        let args = args_json.and_then(|json| serde_json::from_str(&json).ok());

        let status_str: String = row.get(3)?;
        let status = RunStatus::parse(&status_str).unwrap_or(RunStatus::Failed);

        Ok(RunRecord {
            id: row.get(0)?,
            project_id: row.get(1)?,
            timestamp: row.get::<_, i64>(2)? as u64,
            status,
            duration_seconds: row.get(4)?,
            size_bytes: row.get::<_, Option<i64>>(5)?.map(|s| s as u64),
            ottofile_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
            cwd: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
            user: row.get(8)?,
            hostname: row.get(9)?,
            args,
            ended_at: row.get::<_, Option<i64>>(11)?.map(|t| t as u64),
        })
    }

    /// Ensure a project exists in the database, creating it if necessary
    /// Returns the project ID
    fn ensure_project(&self, conn: &rusqlite::Connection, hash: &str, ottofile_path: Option<&PathBuf>) -> Result<i64> {
        // Try to find existing project
        let existing: Option<i64> = conn
            .query_row("SELECT id FROM projects WHERE hash = ?1", params![hash], |row| {
                row.get(0)
            })
            .optional()
            .context("Failed to query for existing project")?;

        if let Some(id) = existing {
            return Ok(id);
        }

        // Create new project
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .context("Failed to get current time")?
            .as_secs();

        conn.execute(
            "INSERT INTO projects (hash, ottofile_path, first_seen, last_seen, run_count)
             VALUES (?1, ?2, ?3, ?4, 0)",
            params![
                hash,
                ottofile_path.map(|p| p.to_string_lossy().to_string()),
                now as i64,
                now as i64,
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_manager() -> Result<(StateManager, TempDir)> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");
        let manager = StateManager::with_db_path(db_path)?;
        Ok((manager, temp_dir))
    }

    #[test]
    fn test_record_run_start() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        let metadata = RunMetadata::minimal(Some(PathBuf::from("/test/otto.yml")), "abc123".to_string(), 1234567890);

        let run_id = manager.record_run_start(&metadata)?;
        assert!(run_id > 0);

        Ok(())
    }

    #[test]
    fn test_record_run_complete() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        let metadata = RunMetadata::minimal(Some(PathBuf::from("/test/otto.yml")), "abc123".to_string(), 1234567890);

        manager.record_run_start(&metadata)?;
        manager.record_run_complete(1234567890, RunStatus::Success, Some(1024))?;

        // Verify the run was updated
        let runs = manager.get_recent_runs(1, None)?;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Success);
        assert_eq!(runs[0].size_bytes, Some(1024));
        assert!(runs[0].duration_seconds.is_some());

        Ok(())
    }

    #[test]
    fn test_get_recent_runs() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        // Create multiple runs
        for i in 0..5 {
            let metadata = RunMetadata::minimal(
                Some(PathBuf::from("/test/otto.yml")),
                "abc123".to_string(),
                1234567890 + i,
            );
            manager.record_run_start(&metadata)?;
        }

        // Get recent runs with limit
        let runs = manager.get_recent_runs(3, None)?;
        assert_eq!(runs.len(), 3);

        // Should be ordered by timestamp descending (newest first)
        assert!(runs[0].timestamp > runs[1].timestamp);
        assert!(runs[1].timestamp > runs[2].timestamp);

        Ok(())
    }

    #[test]
    fn test_get_recent_runs_with_project_filter() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        // Create runs for different projects
        for i in 0..3 {
            let metadata1 = RunMetadata::minimal(
                Some(PathBuf::from("/test/otto.yml")),
                "abc123".to_string(),
                1234567890 + i,
            );
            manager.record_run_start(&metadata1)?;

            let metadata2 = RunMetadata::minimal(
                Some(PathBuf::from("/test/otto.yml")),
                "def456".to_string(),
                1234567890 + i + 100,
            );
            manager.record_run_start(&metadata2)?;
        }

        // Get runs for specific project
        let runs = manager.get_recent_runs(10, Some("abc123"))?;
        assert_eq!(runs.len(), 3);

        // All runs should be for abc123
        // We can verify by checking timestamps match what we inserted
        assert!(runs.iter().all(|r| r.timestamp < 1234567890 + 100));

        Ok(())
    }

    #[test]
    fn test_ensure_project_creates_new() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        manager.db.with_connection(|conn| {
            let project_id1 = manager.ensure_project(conn, "test123", Some(&PathBuf::from("/test/otto.yml")))?;
            assert!(project_id1 > 0);

            // Calling again should return same ID
            let project_id2 = manager.ensure_project(conn, "test123", Some(&PathBuf::from("/test/otto.yml")))?;
            assert_eq!(project_id1, project_id2);

            Ok(())
        })
    }

    #[test]
    fn test_full_metadata_recording() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        let metadata = RunMetadata::full(
            Some(PathBuf::from("/test/otto.yml")),
            "abc123".to_string(),
            1234567890,
            Some(PathBuf::from("/home/user/project")),
            Some("testuser".to_string()),
            Some("testhost".to_string()),
            Some(vec!["build".to_string(), "test".to_string()]),
        );

        manager.record_run_start(&metadata)?;

        let runs = manager.get_recent_runs(1, None)?;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].cwd, Some(PathBuf::from("/home/user/project")));
        assert_eq!(runs[0].user, Some("testuser".to_string()));
        assert_eq!(runs[0].hostname, Some("testhost".to_string()));
        assert_eq!(runs[0].args, Some(vec!["build".to_string(), "test".to_string()]));

        Ok(())
    }

    #[test]
    fn test_try_new_graceful_failure() {
        // This test verifies that try_new() returns None for invalid paths
        // We can't easily test this without mocking, but we can at least verify it compiles
        let _result = StateManager::try_new();
    }
}
