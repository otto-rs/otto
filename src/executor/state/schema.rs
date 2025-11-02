use eyre::Result;
use rusqlite::Connection;

/// SQL schema for the otto database
pub const SCHEMA_VERSION: i64 = 1;

/// Status of a run
#[derive(Debug, Clone, PartialEq)]
pub enum RunStatus {
    Running,
    Success,
    Failed,
}

impl RunStatus {
    pub fn as_str(&self) -> &str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Success => "success",
            RunStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "running" => Some(RunStatus::Running),
            "success" => Some(RunStatus::Success),
            "failed" => Some(RunStatus::Failed),
            _ => None,
        }
    }
}

/// Status of a task
#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl TaskStatus {
    pub fn as_str(&self) -> &str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Skipped => "skipped",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(TaskStatus::Pending),
            "running" => Some(TaskStatus::Running),
            "completed" => Some(TaskStatus::Completed),
            "failed" => Some(TaskStatus::Failed),
            "skipped" => Some(TaskStatus::Skipped),
            _ => None,
        }
    }
}

/// Initialize the database schema
pub fn init_schema(conn: &Connection) -> Result<()> {
    // Schema version table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
        [],
    )?;

    // Projects table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS projects (
            id INTEGER PRIMARY KEY,
            hash TEXT NOT NULL UNIQUE,
            ottofile_path TEXT,
            first_seen INTEGER NOT NULL,
            last_seen INTEGER NOT NULL,
            run_count INTEGER DEFAULT 0
        )",
        [],
    )?;

    // Runs table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS runs (
            id INTEGER PRIMARY KEY,
            project_id INTEGER NOT NULL,
            timestamp INTEGER NOT NULL UNIQUE,
            status TEXT NOT NULL,
            duration_seconds REAL,
            size_bytes INTEGER,
            ottofile_path TEXT,
            cwd TEXT,
            user TEXT,
            hostname TEXT,
            args TEXT,
            ended_at INTEGER,
            FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // Runs indexes
    conn.execute("CREATE INDEX IF NOT EXISTS idx_runs_timestamp ON runs(timestamp)", [])?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status)", [])?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_runs_project ON runs(project_id)", [])?;

    // Tasks table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tasks (
            id INTEGER PRIMARY KEY,
            run_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            status TEXT NOT NULL,
            script_hash TEXT,
            exit_code INTEGER,
            started_at INTEGER,
            ended_at INTEGER,
            duration_seconds REAL,
            stdout_path TEXT,
            stderr_path TEXT,
            script_path TEXT,
            FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // Tasks indexes
    conn.execute("CREATE INDEX IF NOT EXISTS idx_tasks_run ON tasks(run_id)", [])?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_tasks_name ON tasks(name)", [])?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status)", [])?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn test_init_schema() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        init_schema(&conn)?;

        // Verify schema_version table exists
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='schema_version'")?;
        let exists = stmt.exists([])?;
        assert!(exists);

        // Verify projects table exists
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='projects'")?;
        let exists = stmt.exists([])?;
        assert!(exists);

        // Verify runs table exists
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='runs'")?;
        let exists = stmt.exists([])?;
        assert!(exists);

        // Verify tasks table exists
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='tasks'")?;
        let exists = stmt.exists([])?;
        assert!(exists);

        Ok(())
    }

    #[test]
    fn test_run_status_conversions() {
        assert_eq!(RunStatus::Running.as_str(), "running");
        assert_eq!(RunStatus::Success.as_str(), "success");
        assert_eq!(RunStatus::Failed.as_str(), "failed");

        assert_eq!(RunStatus::parse("running"), Some(RunStatus::Running));
        assert_eq!(RunStatus::parse("success"), Some(RunStatus::Success));
        assert_eq!(RunStatus::parse("failed"), Some(RunStatus::Failed));
        assert_eq!(RunStatus::parse("invalid"), None);
    }

    #[test]
    fn test_task_status_conversions() {
        assert_eq!(TaskStatus::Pending.as_str(), "pending");
        assert_eq!(TaskStatus::Running.as_str(), "running");
        assert_eq!(TaskStatus::Completed.as_str(), "completed");
        assert_eq!(TaskStatus::Failed.as_str(), "failed");
        assert_eq!(TaskStatus::Skipped.as_str(), "skipped");

        assert_eq!(TaskStatus::parse("pending"), Some(TaskStatus::Pending));
        assert_eq!(TaskStatus::parse("running"), Some(TaskStatus::Running));
        assert_eq!(TaskStatus::parse("completed"), Some(TaskStatus::Completed));
        assert_eq!(TaskStatus::parse("failed"), Some(TaskStatus::Failed));
        assert_eq!(TaskStatus::parse("skipped"), Some(TaskStatus::Skipped));
        assert_eq!(TaskStatus::parse("invalid"), None);
    }
}
