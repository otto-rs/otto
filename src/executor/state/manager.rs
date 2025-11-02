use eyre::{Context, Result};
use rusqlite::{OptionalExtension, params};
use std::path::PathBuf;
use std::time::SystemTime;

use super::db::DatabaseManager;
use super::metadata::RunMetadata;
use super::schema::{RunStatus, TaskStatus};

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

/// A task record from the database
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub id: i64,
    pub run_id: i64,
    pub name: String,
    pub status: TaskStatus,
    pub script_hash: Option<String>,
    pub exit_code: Option<i32>,
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub duration_seconds: Option<f64>,
    pub stdout_path: Option<PathBuf>,
    pub stderr_path: Option<PathBuf>,
    pub script_path: Option<PathBuf>,
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

    /// Record the start of a task
    /// Returns the database task ID if successful
    pub fn record_task_start(
        &self,
        run_id: i64,
        task_name: &str,
        script_hash: Option<&str>,
        stdout_path: Option<&PathBuf>,
        stderr_path: Option<&PathBuf>,
        script_path: Option<&PathBuf>,
    ) -> Result<i64> {
        let started_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .context("Failed to get current time")?
            .as_secs();

        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO tasks (
                    run_id, name, status, script_hash, started_at,
                    stdout_path, stderr_path, script_path
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    run_id,
                    task_name,
                    TaskStatus::Running.as_str(),
                    script_hash,
                    started_at as i64,
                    stdout_path.map(|p| p.to_string_lossy().to_string()),
                    stderr_path.map(|p| p.to_string_lossy().to_string()),
                    script_path.map(|p| p.to_string_lossy().to_string()),
                ],
            )?;

            Ok(conn.last_insert_rowid())
        })
    }

    /// Record the completion of a task
    pub fn record_task_complete(&self, task_id: i64, exit_code: i32, status: TaskStatus) -> Result<()> {
        let ended_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .context("Failed to get current time")?
            .as_secs();

        self.db.with_connection(|conn| {
            // Get the started_at time to calculate duration
            let started_at: i64 =
                conn.query_row("SELECT started_at FROM tasks WHERE id = ?1", params![task_id], |row| {
                    row.get(0)
                })?;

            let duration_seconds = (ended_at as i64 - started_at) as f64;

            conn.execute(
                "UPDATE tasks
                 SET status = ?1, exit_code = ?2, ended_at = ?3, duration_seconds = ?4
                 WHERE id = ?5",
                params![status.as_str(), exit_code, ended_at as i64, duration_seconds, task_id],
            )?;

            Ok(())
        })
    }

    /// Record that a task was skipped
    pub fn record_task_skipped(&self, run_id: i64, task_name: &str, script_hash: Option<&str>) -> Result<i64> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO tasks (run_id, name, status, script_hash)
                 VALUES (?1, ?2, ?3, ?4)",
                params![run_id, task_name, TaskStatus::Skipped.as_str(), script_hash],
            )?;

            Ok(conn.last_insert_rowid())
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

    /// Get tasks for a specific run
    pub fn get_run_tasks(&self, run_id: i64) -> Result<Vec<TaskRecord>> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, run_id, name, status, script_hash, exit_code,
                        started_at, ended_at, duration_seconds,
                        stdout_path, stderr_path, script_path
                 FROM tasks
                 WHERE run_id = ?1
                 ORDER BY started_at ASC",
            )?;

            let rows = stmt.query_map(params![run_id], Self::row_to_task_record)?;

            rows.collect::<Result<Vec<_>, _>>().context("Failed to fetch tasks")
        })
    }

    /// Get task history by task name across all runs
    pub fn get_task_history(&self, task_name: &str, limit: usize) -> Result<Vec<TaskRecord>> {
        self.db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, run_id, name, status, script_hash, exit_code,
                        started_at, ended_at, duration_seconds,
                        stdout_path, stderr_path, script_path
                 FROM tasks
                 WHERE name = ?1
                 ORDER BY started_at DESC
                 LIMIT ?2",
            )?;

            let rows = stmt.query_map(params![task_name, limit as i64], Self::row_to_task_record)?;

            rows.collect::<Result<Vec<_>, _>>()
                .context("Failed to fetch task history")
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

    /// Helper to convert a database row to a TaskRecord
    fn row_to_task_record(row: &rusqlite::Row) -> rusqlite::Result<TaskRecord> {
        let status_str: String = row.get(3)?;
        let status = TaskStatus::parse(&status_str).unwrap_or(TaskStatus::Failed);

        Ok(TaskRecord {
            id: row.get(0)?,
            run_id: row.get(1)?,
            name: row.get(2)?,
            status,
            script_hash: row.get(4)?,
            exit_code: row.get(5)?,
            started_at: row.get::<_, Option<i64>>(6)?.map(|t| t as u64),
            ended_at: row.get::<_, Option<i64>>(7)?.map(|t| t as u64),
            duration_seconds: row.get(8)?,
            stdout_path: row.get::<_, Option<String>>(9)?.map(PathBuf::from),
            stderr_path: row.get::<_, Option<String>>(10)?.map(PathBuf::from),
            script_path: row.get::<_, Option<String>>(11)?.map(PathBuf::from),
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

    #[test]
    fn test_record_task_start() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        let metadata = RunMetadata::minimal(Some(PathBuf::from("/test/otto.yml")), "abc123".to_string(), 1234567890);
        let run_id = manager.record_run_start(&metadata)?;

        let task_id = manager.record_task_start(
            run_id,
            "test-task",
            Some("hash123"),
            Some(&PathBuf::from("/tmp/stdout.log")),
            Some(&PathBuf::from("/tmp/stderr.log")),
            Some(&PathBuf::from("/tmp/script.sh")),
        )?;

        assert!(task_id > 0);

        // Verify task was recorded
        let tasks = manager.get_run_tasks(run_id)?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "test-task");
        assert_eq!(tasks[0].status, TaskStatus::Running);
        assert_eq!(tasks[0].script_hash, Some("hash123".to_string()));

        Ok(())
    }

    #[test]
    fn test_record_task_complete() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        let metadata = RunMetadata::minimal(Some(PathBuf::from("/test/otto.yml")), "abc123".to_string(), 1234567890);
        let run_id = manager.record_run_start(&metadata)?;

        let task_id = manager.record_task_start(run_id, "test-task", None, None, None, None)?;
        manager.record_task_complete(task_id, 0, TaskStatus::Completed)?;

        // Verify task was updated
        let tasks = manager.get_run_tasks(run_id)?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, TaskStatus::Completed);
        assert_eq!(tasks[0].exit_code, Some(0));
        assert!(tasks[0].ended_at.is_some());
        assert!(tasks[0].duration_seconds.is_some());

        Ok(())
    }

    #[test]
    fn test_record_task_failed() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        let metadata = RunMetadata::minimal(Some(PathBuf::from("/test/otto.yml")), "abc123".to_string(), 1234567890);
        let run_id = manager.record_run_start(&metadata)?;

        let task_id = manager.record_task_start(run_id, "test-task", None, None, None, None)?;
        manager.record_task_complete(task_id, 1, TaskStatus::Failed)?;

        // Verify task was updated with failure
        let tasks = manager.get_run_tasks(run_id)?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, TaskStatus::Failed);
        assert_eq!(tasks[0].exit_code, Some(1));

        Ok(())
    }

    #[test]
    fn test_record_task_skipped() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        let metadata = RunMetadata::minimal(Some(PathBuf::from("/test/otto.yml")), "abc123".to_string(), 1234567890);
        let run_id = manager.record_run_start(&metadata)?;

        let task_id = manager.record_task_skipped(run_id, "test-task", Some("hash123"))?;
        assert!(task_id > 0);

        // Verify task was recorded as skipped
        let tasks = manager.get_run_tasks(run_id)?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "test-task");
        assert_eq!(tasks[0].status, TaskStatus::Skipped);
        assert_eq!(tasks[0].script_hash, Some("hash123".to_string()));

        Ok(())
    }

    #[test]
    fn test_get_run_tasks() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        let metadata = RunMetadata::minimal(Some(PathBuf::from("/test/otto.yml")), "abc123".to_string(), 1234567890);
        let run_id = manager.record_run_start(&metadata)?;

        // Create multiple tasks
        let task_id1 = manager.record_task_start(run_id, "task-1", None, None, None, None)?;
        let task_id2 = manager.record_task_start(run_id, "task-2", None, None, None, None)?;
        let task_id3 = manager.record_task_start(run_id, "task-3", None, None, None, None)?;

        manager.record_task_complete(task_id1, 0, TaskStatus::Completed)?;
        manager.record_task_complete(task_id2, 1, TaskStatus::Failed)?;
        manager.record_task_complete(task_id3, 0, TaskStatus::Completed)?;

        // Get all tasks for the run
        let tasks = manager.get_run_tasks(run_id)?;
        assert_eq!(tasks.len(), 3);

        // Tasks should be ordered by started_at
        assert_eq!(tasks[0].name, "task-1");
        assert_eq!(tasks[1].name, "task-2");
        assert_eq!(tasks[2].name, "task-3");

        Ok(())
    }

    #[test]
    fn test_get_task_history() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        // Create multiple runs with the same task name
        for i in 0..5 {
            let metadata = RunMetadata::minimal(
                Some(PathBuf::from("/test/otto.yml")),
                "abc123".to_string(),
                1234567890 + i,
            );
            let run_id = manager.record_run_start(&metadata)?;

            let task_id = manager.record_task_start(run_id, "build", None, None, None, None)?;
            manager.record_task_complete(task_id, 0, TaskStatus::Completed)?;

            // Add a small sleep to ensure different timestamps
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Get history for the build task
        let history = manager.get_task_history("build", 3)?;
        assert_eq!(history.len(), 3);

        // Should be ordered by started_at descending (newest first)
        // Use >= instead of > since timestamps might be the same in fast execution
        assert!(history[0].started_at >= history[1].started_at);
        assert!(history[1].started_at >= history[2].started_at);

        // All should be the same task name
        assert!(history.iter().all(|t| t.name == "build"));

        Ok(())
    }

    #[test]
    fn test_task_with_all_fields() -> Result<()> {
        let (manager, _temp_dir) = create_test_manager()?;

        let metadata = RunMetadata::minimal(Some(PathBuf::from("/test/otto.yml")), "abc123".to_string(), 1234567890);
        let run_id = manager.record_run_start(&metadata)?;

        let task_id = manager.record_task_start(
            run_id,
            "complex-task",
            Some("script_hash_123"),
            Some(&PathBuf::from("/tmp/stdout.log")),
            Some(&PathBuf::from("/tmp/stderr.log")),
            Some(&PathBuf::from("/tmp/script.sh")),
        )?;

        manager.record_task_complete(task_id, 0, TaskStatus::Completed)?;

        let tasks = manager.get_run_tasks(run_id)?;
        assert_eq!(tasks.len(), 1);

        let task = &tasks[0];
        assert_eq!(task.name, "complex-task");
        assert_eq!(task.script_hash, Some("script_hash_123".to_string()));
        assert_eq!(task.stdout_path, Some(PathBuf::from("/tmp/stdout.log")));
        assert_eq!(task.stderr_path, Some(PathBuf::from("/tmp/stderr.log")));
        assert_eq!(task.script_path, Some(PathBuf::from("/tmp/script.sh")));
        assert_eq!(task.exit_code, Some(0));
        assert!(task.started_at.is_some());
        assert!(task.ended_at.is_some());
        assert!(task.duration_seconds.is_some());

        Ok(())
    }
}
