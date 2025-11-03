use eyre::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::migrations::migrate;

/// Database manager for Otto's SQLite database
pub struct DatabaseManager {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
}

impl DatabaseManager {
    /// Create a new DatabaseManager
    ///
    /// This will:
    /// 1. Create the database file at the specified path
    /// 2. Enable WAL mode for better concurrency
    /// 3. Run schema migrations
    pub fn new(db_path: PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create database directory")?;
        }

        let conn = Connection::open(&db_path).context(format!("Failed to open database at {}", db_path.display()))?;

        // Enable WAL mode for better concurrency
        conn.pragma_update(None, "journal_mode", "WAL")
            .context("Failed to enable WAL mode")?;

        // Enable foreign keys
        conn.pragma_update(None, "foreign_keys", "ON")
            .context("Failed to enable foreign keys")?;

        // Run migrations
        migrate(&conn).context("Failed to run database migrations")?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path,
        })
    }

    /// Open the database at the default Otto location (~/.otto/otto.db)
    pub fn open_default() -> Result<Self> {
        let db_path = Self::default_db_path()?;
        Self::new(db_path)
    }

    /// Get the default database path (~/.otto/otto.db)
    /// Can be overridden with OTTO_DB_PATH environment variable
    pub fn default_db_path() -> Result<PathBuf> {
        if let Ok(db_path) = std::env::var("OTTO_DB_PATH") {
            return Ok(PathBuf::from(db_path));
        }

        let home = std::env::var("HOME").context("Failed to get HOME environment variable")?;
        Ok(PathBuf::from(home).join(".otto").join("otto.db"))
    }

    /// Get the database path
    pub fn path(&self) -> &Path {
        &self.db_path
    }

    /// Execute a closure with access to the database connection
    ///
    /// This is the primary way to interact with the database.
    /// The connection is locked for the duration of the closure.
    pub fn with_connection<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| eyre::eyre!("Failed to lock database connection: {}", e))?;
        f(&conn)
    }

    pub fn health_check(&self) -> Result<()> {
        self.with_connection(|conn| {
            conn.query_row("SELECT 1", [], |_| Ok(()))?;
            Ok(())
        })
    }

    /// Get database statistics
    pub fn stats(&self) -> Result<DatabaseStats> {
        self.with_connection(|conn| {
            let project_count: i64 = conn.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))?;

            let run_count: i64 = conn.query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))?;

            let task_count: i64 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))?;

            Ok(DatabaseStats {
                project_count,
                run_count,
                task_count,
            })
        })
    }
}

/// Database statistics
#[derive(Debug, Clone)]
pub struct DatabaseStats {
    pub project_count: i64,
    pub run_count: i64,
    pub task_count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_new_database() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        let _db = DatabaseManager::new(db_path.clone())?;
        assert!(db_path.exists());

        Ok(())
    }

    #[test]
    fn test_wal_mode_enabled() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        let db = DatabaseManager::new(db_path)?;

        db.with_connection(|conn| {
            let journal_mode: String = conn.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
            assert_eq!(journal_mode.to_lowercase(), "wal");
            Ok(())
        })?;

        Ok(())
    }

    #[test]
    fn test_foreign_keys_enabled() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        let db = DatabaseManager::new(db_path)?;

        db.with_connection(|conn| {
            let foreign_keys: i64 = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
            assert_eq!(foreign_keys, 1);
            Ok(())
        })?;

        Ok(())
    }

    #[test]
    fn test_health_check() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        let db = DatabaseManager::new(db_path)?;
        db.health_check()?;

        Ok(())
    }

    #[test]
    fn test_stats() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        let db = DatabaseManager::new(db_path)?;
        let stats = db.stats()?;

        // New database should have zero records
        assert_eq!(stats.project_count, 0);
        assert_eq!(stats.run_count, 0);
        assert_eq!(stats.task_count, 0);

        Ok(())
    }

    #[test]
    fn test_with_connection() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        let db = DatabaseManager::new(db_path)?;

        // Test that we can execute queries through with_connection
        db.with_connection(|conn| {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))?;
            assert_eq!(count, 0);
            Ok(())
        })?;

        Ok(())
    }

    #[test]
    fn test_schema_initialized() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        let db = DatabaseManager::new(db_path)?;

        db.with_connection(|conn| {
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;

            assert!(tables.contains(&"projects".to_string()));
            assert!(tables.contains(&"runs".to_string()));
            assert!(tables.contains(&"tasks".to_string()));
            assert!(tables.contains(&"schema_version".to_string()));

            Ok(())
        })?;

        Ok(())
    }
}
