# SQLite Integration Architecture

Otto uses a hybrid storage model combining SQLite for metadata with filesystem storage for artifacts. This provides fast querying while keeping scripts and logs inspectable.

## Design Principles

### What Stays On Filesystem ✅
- **Scripts** (`.cache/<task>/<hash>`) - Inspectable with `cat`
- **Logs** (`stdout.log`, `stderr.log`) - Tailable with `tail -f`
- **Outputs** (`output.json`) - Parseable with standard tools
- **Artifacts** (task-generated files) - Direct filesystem access

### What Goes In Database ✅
- **Run metadata** (timestamp, status, duration, user, etc.)
- **Task state** (pending/running/completed/failed/skipped)
- **Relationships** (task dependencies, project associations)
- **Paths** to filesystem artifacts (not content)
- **Metrics** (counts, durations, exit codes, sizes)

### Non-Negotiables

- ✅ Scripts remain inspectable with `cat`
- ✅ Logs remain tailable with `tail -f`
- ✅ Database is optional (graceful degradation)
- ✅ Zero breaking changes to existing workflows
- ✅ Performance is same or better

## Database Schema

### Projects Table

Tracks unique Otto projects (identified by ottofile hash):

```sql
CREATE TABLE projects (
    id INTEGER PRIMARY KEY,
    hash TEXT NOT NULL UNIQUE,        -- Project identifier (e.g., "abc123")
    ottofile_path TEXT,                -- Canonical path to otto.yml
    first_seen INTEGER NOT NULL,       -- Unix timestamp of first run
    last_seen INTEGER NOT NULL,        -- Unix timestamp of most recent run
    run_count INTEGER DEFAULT 0        -- Total number of runs
);
```

**Indexes:**
- Primary key on `id`
- Unique index on `hash`

### Runs Table

Records each Otto execution:

```sql
CREATE TABLE runs (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL,
    timestamp INTEGER NOT NULL UNIQUE, -- Directory name, also run start time
    status TEXT NOT NULL,              -- 'running', 'success', 'failed'
    duration_seconds REAL,             -- NULL if still running
    size_bytes INTEGER,                -- Total run directory size
    ottofile_path TEXT,                -- Path at run time (may differ)
    cwd TEXT,                          -- Working directory
    user TEXT,                         -- Username
    hostname TEXT,                     -- Host where it ran
    args TEXT,                         -- JSON-serialized command args
    ended_at INTEGER,                  -- Unix timestamp when completed
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);
```

**Indexes:**
- Primary key on `id`
- Unique index on `timestamp`
- Index on `status` (for filtering)
- Index on `project_id` (for project queries)

**Foreign Keys:**
- Cascade delete: Deleting a project removes all its runs

### Tasks Table

Tracks individual task executions within runs:

```sql
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY,
    run_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,              -- 'pending', 'running', 'completed', 'failed', 'skipped'
    script_hash TEXT,                  -- Hash of script content (for cache tracking)
    exit_code INTEGER,                 -- Process exit code
    started_at INTEGER,                -- Unix timestamp
    ended_at INTEGER,                  -- Unix timestamp
    duration_seconds REAL,             -- Execution time
    stdout_path TEXT,                  -- Relative path from ~/.otto/
    stderr_path TEXT,                  -- Relative path from ~/.otto/
    script_path TEXT,                  -- Relative path from ~/.otto/
    FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
);
```

**Indexes:**
- Primary key on `id`
- Index on `run_id` (for run queries)
- Index on `name` (for task history)
- Index on `status` (for filtering)

**Foreign Keys:**
- Cascade delete: Deleting a run removes all its tasks

### Schema Version Table

Tracks database migrations:

```sql
CREATE TABLE schema_version (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
);
```

## Component Architecture

### StateManager (`src/executor/state/manager.rs`)

Central component for database operations:

```rust
pub struct StateManager {
    db: DatabaseManager,  // Connection pooling and transactions
}
```

**Key Methods:**

**Recording:**
- `record_run_start()` - Create run record, mark as running
- `record_run_complete()` - Update run with final status and metrics
- `record_task_start()` - Record task beginning
- `record_task_complete()` - Record task completion with exit code
- `record_task_skipped()` - Mark task as skipped

**Querying:**
- `get_recent_runs()` - Fetch last N runs, optionally filtered
- `get_run_tasks()` - Get all tasks for a specific run
- `get_task_history()` - Get execution history for a task name
- `get_overall_stats()` - Aggregate statistics
- `get_task_stats()` - Task-specific statistics

**Cleanup:**
- `find_old_runs()` - Query runs matching retention policy
- `delete_run()` - Remove run from database and filesystem

### DatabaseManager (`src/executor/state/db.rs`)

Low-level database operations:

```rust
pub struct DatabaseManager {
    db_path: PathBuf,
}
```

**Features:**
- **WAL Mode**: Write-Ahead Logging for better concurrency
- **Foreign Keys**: Enabled for referential integrity
- **Connection Pooling**: Efficient connection reuse
- **Transactions**: Atomic operations
- **Health Checks**: Automatic integrity verification

### Migration Framework (`src/executor/state/migrations.rs`)

Handles schema evolution:

```rust
pub fn migrate(conn: &Connection) -> Result<()>
```

**Features:**
- **Versioned migrations**: Track applied schema changes
- **Idempotent**: Safe to run multiple times
- **Transactional**: All-or-nothing application
- **Rollback support**: Can revert failed migrations

## Data Flow

### Run Execution

```
1. User runs: otto build test
   ↓
2. Workspace::init()
   ├─ Create run directory: ~/.otto/otto-abc123/1730561025/
   └─ Save run.yaml (metadata)
   ↓
3. StateManager::record_run_start()
   ├─ Insert project (if new)
   ├─ Insert run record (status: running)
   └─ Return run_id
   ↓
4. Task execution loop
   ├─ StateManager::record_task_start()
   ├─ Execute task → Generate logs/outputs
   ├─ StateManager::record_task_complete()
   └─ Repeat for each task
   ↓
5. StateManager::record_run_complete()
   ├─ Calculate run size
   ├─ Update run record (status: success/failed)
   └─ Update project last_seen
```

### History Query

```
1. User runs: otto history
   ↓
2. StateManager::get_recent_runs(limit=20)
   ↓
3. SQL Query:
   SELECT r.*, p.hash
   FROM runs r
   JOIN projects p ON r.project_id = p.id
   ORDER BY r.timestamp DESC
   LIMIT 20
   ↓
4. Format and display results
```

### Cleanup Operation

```
1. User runs: otto clean --keep-days 30 --keep-last 10
   ↓
2. StateManager::find_old_runs()
   ├─ Query all runs (ORDER BY timestamp DESC)
   ├─ Keep first 10 (most recent)
   ├─ Filter remainder by age
   └─ Return list of runs to delete
   ↓
3. For each run:
   ├─ StateManager::delete_run()
   ├─ Delete database record
   ├─ Delete filesystem directory
   └─ Update project run_count
```

## Filesystem Layout

```
~/.otto/
├── otto.db                     # SQLite database (Phase 2+)
├── otto-<project-hash>/        # Per-project directories
│   ├── <timestamp>/            # Individual run directories
│   │   ├── run.yaml           # Run metadata (Phase 1, kept for backward compat)
│   │   ├── tasks/             # Task execution data
│   │   │   ├── <task-name>/
│   │   │   │   ├── script.sh       # Generated script
│   │   │   │   ├── stdout.log      # Task output
│   │   │   │   ├── stderr.log      # Task errors
│   │   │   │   └── output.json     # Task result
│   │   └── ...
└── .cache/                     # Script cache (future)
```

**Database stores:**
- Metadata only (see schema above)

**Filesystem stores:**
- All actual files (scripts, logs, outputs)
- Database contains relative paths to these files

## Performance Characteristics

### Database Operations

| Operation | Typical Time | Notes |
|-----------|--------------|-------|
| `record_run_start()` | <5ms | Single INSERT |
| `record_run_complete()` | <5ms | Single UPDATE |
| `record_task_start()` | <5ms | Single INSERT |
| `get_recent_runs(20)` | <10ms | Indexed query |
| `get_task_history(100)` | <20ms | Indexed query |
| `find_old_runs()` | <50ms | Full table scan with filtering |
| `delete_run()` | ~100ms | Database + filesystem I/O |

### Scalability

| Dataset Size | Query Performance | Notes |
|--------------|-------------------|-------|
| 100 runs | <10ms | Instant |
| 1,000 runs | <20ms | Very fast |
| 10,000 runs | <100ms | Fast |
| 100,000 runs | <500ms | Acceptable |

**Bottlenecks:**
- Filesystem I/O (deletion, size calculation)
- Not database queries (well-indexed)

### Storage Overhead

- **Database size**: ~2-5 KB per run
- **10,000 runs**: ~20-50 MB database
- **Minimal compared to**: Actual run artifacts (MBs-GBs)

## Concurrency Model

### WAL Mode Benefits

Write-Ahead Logging provides:
- **Concurrent reads**: Multiple queries don't block
- **Writer isolation**: Writes don't block readers
- **Crash recovery**: Automatic on next connection
- **Performance**: Faster commits

### Lock Behavior

```
READ operations:  No lock (WAL allows concurrent reads)
WRITE operations: Brief lock (milliseconds)
```

**Implications:**
- Multiple `otto history` commands can run simultaneously
- Recording task completion doesn't block queries
- Parallel task execution doesn't cause contention

## Error Handling

### Database Unavailable

```rust
if let Some(manager) = StateManager::try_new() {
    // Use database-backed operations
} else {
    // Fall back to filesystem-only mode
    eprintln!("Warning: Database unavailable, using filesystem fallback");
}
```

**Graceful degradation:**
- `history`/`stats` commands report error but don't crash
- `clean` command falls back to filesystem scan
- Run execution continues without database

### Database Corruption

1. **Detection**: Health check on connection
2. **Response**: Fall back to in-memory database
3. **Recovery**: User can delete and recreate

```bash
# Manual recovery
rm ~/.otto/otto.db
otto history  # Automatically recreates database
```

### Partial Writes

**Scenario**: Process killed during `record_run_complete()`

**Result:**
- Run remains marked as "running"
- Next query shows stale state

**Recovery:**
- User can manually clean with `otto clean`
- Future enhancement: Detect and mark stale runs

## Testing Strategy

### Unit Tests (`src/executor/state/manager.rs`)

- Test each StateManager method
- Use in-memory database (`:memory:`)
- Verify schema constraints
- Test error conditions

### Integration Tests (`tests/cleanup_integration_test.rs`)

- End-to-end command testing
- Database + filesystem interaction
- Graceful fallback scenarios
- Project filtering and retention policies

**Coverage:**
- 11 unit tests for database operations
- 8 integration tests for CLI commands
- 100% coverage of new Phase 5 functionality

## Future Enhancements

### Action Cache (Post-1.0)
- Cache task results by (script_hash, inputs_hash)
- Skip unchanged tasks automatically
- Like Bazel's action cache

### Web Dashboard (Post-1.0)
- Real-time execution view
- Historical trends and charts
- Failure analysis tools

### Multi-User Support (Post-1.0)
- PostgreSQL backend option
- Shared execution history
- Team dashboards

## Migration Path

See [Migration Guide](../migration-guide.md) for upgrading from file-only to database-backed storage.

## Related Documentation

- [History Command](../commands/history.md)
- [Stats Command](../commands/stats.md)
- [Clean Command](../commands/clean.md)
- [Implementation Plan](../sqlite-implementation-plan.md)

