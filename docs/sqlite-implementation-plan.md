# SQLite Hybrid Storage - Implementation Plan

**Decision**: Add SQLite for metadata while keeping scripts/logs on filesystem

**Why**: Preserves inspectability while adding queryability, state management, and observability

---

## Phase 1: SQLite Infrastructure

### Goal
Add SQLite database layer without changing any existing functionality

### Deliverables
- [ ] Add `rusqlite` and `tokio-rusqlite` to Cargo.toml
- [ ] Create `src/executor/state/` module
- [ ] Database connection management with WAL mode
- [ ] Schema migration framework
- [ ] Initial schema (projects, runs, tasks tables)
- [ ] All existing tests still pass

### What Gets Built
- Database manager that opens `~/.otto/otto.db`
- Schema versioning table
- Migration system for future schema changes
- Connection pooling for async operations

### Success Criteria
- Database initializes on first run
- Schema applies correctly
- No impact on existing functionality
- Unit tests for database operations

---

## Phase 2: Run Tracking

### Goal
Record run metadata in database (non-intrusive)

### Deliverables
- [ ] Record run start/complete in database
- [ ] Store run metadata (timestamp, args, ottofile, user, etc.)
- [ ] Track run status (running/success/failed)
- [ ] Query API to retrieve run history
- [ ] Optional database (fallback to in-memory if DB unavailable)

### What Gets Built
- `StateManager::record_run_start()`
- `StateManager::record_run_complete()`
- `StateManager::get_recent_runs()`
- Integration with TaskScheduler
- Graceful fallback if database fails

### Success Criteria
- Every run recorded in database
- Run history queryable
- System works even if DB is missing/locked
- Performance unchanged

---

## Phase 3: Task State Tracking

### Goal
Track individual task execution state and results

### Deliverables
- [ ] Record task start/complete/skip in database
- [ ] Store task metadata (exit code, duration, paths to logs)
- [ ] Track task dependencies
- [ ] Store paths to scripts/logs (not content)
- [ ] Atomic state updates

### What Gets Built
- `StateManager::record_task_start()`
- `StateManager::record_task_complete()`
- `StateManager::record_task_skipped()`
- Task dependency tracking
- Relative paths to filesystem artifacts

### Success Criteria
- All task state changes recorded atomically
- Task history queryable by name
- File paths stored correctly
- Concurrent task updates don't conflict

---

## Phase 4: Query Commands

### Goal
Add CLI commands to query execution history

### Deliverables
- [ ] `otto history` - Show recent runs
- [ ] `otto history <task>` - Show specific task history
- [ ] `otto stats` - Show overall statistics
- [ ] `otto stats <task>` - Show task-specific stats
- [ ] Pretty-printed table output
- [ ] Optional JSON output format

### What Gets Built
- New CLI subcommands
- Query API methods
- Formatted output (tables)
- Statistics calculations (success rate, avg duration, etc.)

### Success Criteria
- Commands show accurate data
- Performance <100ms for recent history
- Output is readable and useful
- JSON export works for scripting

---

## Phase 5: Cleanup & Retention

### Goal
Automated cleanup of old runs

### Deliverables
- [ ] `otto clean --keep <days>` command
- [ ] Query old runs from database
- [ ] Delete both DB records and filesystem directories
- [ ] Dry-run mode (`--dry-run`)
- [ ] Orphaned cache detection
- [ ] Report disk space freed

### What Gets Built
- `StateManager::find_old_runs()`
- `StateManager::delete_run()`
- `StateManager::find_orphaned_cache()`
- Directory deletion logic
- Size calculation utilities

### Success Criteria
- Cleanup removes both DB and filesystem data
- Dry-run shows what would be deleted
- Orphaned cache entries detected correctly
- Disk space actually freed

---

## Phase 6: Documentation & Polish

### Goal
Production-ready release

### Deliverables
- [ ] User documentation for new commands
- [ ] Migration guide (optional upgrade)
- [ ] Architecture documentation
- [ ] Performance benchmarks
- [ ] Example use cases
- [ ] Error handling review

### What Gets Built
- README updates
- Tutorial for new features
- Troubleshooting guide
- Release notes

### Success Criteria
- Users understand new features
- Clear upgrade path documented
- All error cases handled gracefully
- Performance verified

---

## Key Principles

### What Stays On Filesystem
- ✅ Scripts (`.cache/<task>/<hash>`)
- ✅ Logs (`stdout.log`, `stderr.log`)
- ✅ Outputs (`output.json`)
- ✅ Artifacts (any task-generated files)

### What Goes In Database
- ✅ Run metadata (timestamp, status, duration)
- ✅ Task state (pending/running/completed/failed)
- ✅ Relationships (task dependencies)
- ✅ Paths to filesystem artifacts (not content)
- ✅ Metrics (counts, durations, exit codes)

### Non-Negotiables
- Scripts remain inspectable with `cat`
- Logs remain tailable with `tail -f`
- Database is optional (graceful degradation)
- Zero breaking changes to existing workflows
- Performance is same or better

---

## Phase Dependencies

| Phase | Focus | Depends On |
|-------|-------|------------|
| 1 | SQLite setup | None |
| 2 | Run tracking | Phase 1 |
| 3 | Task tracking | Phase 2 |
| 4 | Query commands | Phase 3 |
| 5 | Cleanup | Phase 4 |
| 6 | Polish | Phase 5 |

---

## Risk Mitigation

### Database Corruption
- **Risk**: SQLite file gets corrupted
- **Mitigation**: WAL mode, integrity checks, fallback to in-memory

### Performance Regression
- **Risk**: Database writes slow down execution
- **Mitigation**: Async writes, benchmarks, optional database

### Migration Failures
- **Risk**: Schema upgrades fail
- **Mitigation**: Transactional migrations, rollback support, backups

### Complexity Creep
- **Risk**: Code becomes harder to maintain
- **Mitigation**: Clear abstractions, comprehensive tests, documentation

---

## Post-1.0 Features (Future)

### Action Cache
- Cache task results by (script_hash, inputs_hash)
- Skip tasks if inputs haven't changed
- Like Bazel's action cache

### Web Dashboard
- Real-time task execution view
- Historical trends
- Failure analysis

### Metrics Export
- Prometheus format
- JSON export for monitoring systems
- Integration with observability tools

### Multi-User Support
- PostgreSQL backend option
- Shared execution history
- Team dashboards

---

## Success Definition

Otto 1.0 with hybrid storage is successful when:

1. **Users can query history**: "Show me the last 10 runs"
2. **Users can analyze failures**: "Why does task X keep failing?"
3. **Users can clean up**: "Delete runs older than 30 days"
4. **Scripts stay inspectable**: `cat ~/.otto/.../script.sh` still works
5. **Zero breaking changes**: Existing otto.yml files work unchanged
6. **Performance maintained**: Execution time is same or better

---

## Open Questions

1. **Default retention period**: 30 days? 90 days? Unlimited?
2. **Database location**: `~/.otto/otto.db` or per-project?
3. **Metrics to track**: What statistics are most useful?
4. **Export formats**: CSV? JSON? Prometheus?
5. **Import old runs**: Scan existing directories to populate DB?

---

## Decision Record

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Storage** | Hybrid (files + SQLite) | Keep inspectability, add queryability |
| **Database** | Single `~/.otto/otto.db` | Simpler management, cross-project queries |
| **Paths** | Relative to `~/.otto/` | Portable, handles HOME changes |
| **Mode** | WAL | Better concurrency, crash recovery |
| **Required** | No (optional) | Graceful degradation, no hard dependency |

---

## Next Steps

1. **Review this plan** - Get feedback from maintainers
2. **Start Phase 1** - Add SQLite infrastructure
3. **Iterate** - Adjust based on implementation learnings
4. **Ship** - Release Otto 1.0 with hybrid storage

