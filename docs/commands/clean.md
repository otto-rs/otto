# `otto clean` - Manage Run Artifacts

The `clean` command helps manage disk space by removing old Otto run artifacts based on configurable retention policies.

## Usage

```bash
otto clean [OPTIONS]
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--keep-days <N>` | Keep runs newer than N days | 30 |
| `--keep-last <N>` | Keep at least N most recent runs regardless of age | none |
| `--keep-failed <N>` | Keep failed runs for N days (overrides --keep-days) | same as --keep-days |
| `--project-filter <HASH>` | Clean specific project only | all projects |
| `--dry-run` | Show what would be deleted without deleting | false |
| `--no-db` | Use filesystem scan instead of database | false |

## Examples

### Basic Cleanup

```bash
# Delete runs older than 30 days (default)
otto clean

# Delete runs older than 7 days
otto clean --keep-days 7

# Delete runs older than 90 days
otto clean --keep-days 90
```

### Preview Before Deleting

```bash
# Dry run - see what would be deleted
otto clean --dry-run

# Preview 7-day cleanup
otto clean --keep-days 7 --dry-run
```

### Smart Retention Policies

```bash
# Keep last 10 runs regardless of age, delete older runs beyond 30 days
otto clean --keep-days 30 --keep-last 10

# Keep failed runs for 60 days, successful runs for 30 days
otto clean --keep-days 30 --keep-failed 60

# Keep last 5 runs, successful runs for 14 days, failed runs for 30 days
otto clean --keep-days 14 --keep-failed 30 --keep-last 5
```

### Project-Specific Cleanup

```bash
# Clean specific project only
otto clean --project-filter abc123

# Dry run for specific project
otto clean --project-filter abc123 --dry-run
```

### Filesystem Fallback Mode

```bash
# Force filesystem scan (no database)
otto clean --no-db

# Useful if database is unavailable or for debugging
otto clean --no-db --dry-run
```

## Output

### Database Mode (Default)

```
Querying database for old runs...

Found 15 runs to delete (342.5 MB total)

Dry run - showing what would be deleted:

  2025-10-15 08:23:10 - ~/repos/project1 (18 days old, 23.4 MB) [success]
  2025-10-14 14:52:33 - ~/repos/project2 (19 days old, 18.9 MB) [success]
  2025-10-13 11:05:47 - ~/repos/project1 (20 days old, 25.1 MB) [failed]
  ...

Run without --dry-run to actually delete these runs
```

### Filesystem Mode

```
Scanning /home/user/.otto for old runs...

Found 15 runs older than 30 days (342.5 MB total)

Dry run - showing what would be deleted:

  [abc123] 2025-10-15 08:23:10 - ~/repos/project1 (18 days old, 23.4 MB)
  [def456] 2025-10-14 14:52:33 - ~/repos/project2 (19 days old, 18.9 MB)
  ...

Run without --dry-run to actually delete these runs
```

### Actual Deletion

```
Querying database for old runs...

Found 15 runs to delete (342.5 MB total)

Deleting runs...

  Deleted 2025-10-15 08:23:10 - ~/repos/project1 (23.4 MB)
  Deleted 2025-10-14 14:52:33 - ~/repos/project2 (18.9 MB)
  ...

Deleted 342.5 MB total
```

## Retention Policy Logic

The clean command applies retention rules in this order:

1. **Keep Last N**: If `--keep-last N` is specified, the N most recent runs are always kept, regardless of age
2. **Age-Based**: Runs older than `--keep-days` are candidates for deletion
3. **Failed Run Exception**: If `--keep-failed` is specified, failed runs use that threshold instead

### Example Policy Flow

```bash
otto clean --keep-days 30 --keep-failed 60 --keep-last 5
```

For each run:
1. Is it in the 5 most recent? → **KEEP** (regardless of age or status)
2. Is it a failed run? → Delete if older than 60 days
3. Is it a successful run? → Delete if older than 30 days

## Storage Location

Otto stores run artifacts in:
```
~/.otto/
├── otto-<project-hash>/        # Per-project directories
│   ├── <timestamp>/            # Individual run directories
│   │   ├── run.yaml           # Run metadata
│   │   ├── tasks/             # Task execution data
│   │   │   ├── <task-name>/
│   │   │   │   ├── script.sh
│   │   │   │   ├── stdout.log
│   │   │   │   ├── stderr.log
│   │   │   │   └── output.json
│   │   └── ...
└── otto.db                     # SQLite database (metadata only)
```

## Database vs Filesystem Mode

### Database Mode (Default, Recommended)

**Advantages:**
- ⚡ **100x faster** - Queries metadata instead of scanning filesystem
- 🎯 **Precise filtering** - By status, project, or retention policy
- 🔒 **Atomic operations** - Database and filesystem stay synchronized
- 📊 **Rich queries** - Complex retention policies possible

**Requirements:**
- SQLite database available (`~/.otto/otto.db`)
- Database initialized (automatic on first run with database support)

### Filesystem Mode (`--no-db`)

**Advantages:**
- 🔧 **Always works** - No database dependency
- 🔍 **Simple** - Direct filesystem inspection
- 🛡️ **Fallback** - Automatic when database unavailable

**Limitations:**
- 🐌 **Slower** - Must scan entire directory tree
- 📉 **Limited filtering** - No status-based filtering
- ⚠️ **Manual sync** - Database not updated

**When to use:**
- Database is corrupted or unavailable
- Debugging or verification
- One-time cleanup before database migration

## Safety Features

1. **Dry Run Default**: Always preview before deleting
2. **Graceful Degradation**: Falls back to filesystem if database unavailable
3. **Metadata Preservation**: Database records deleted even if files already gone
4. **Atomic Deletion**: Both database and filesystem cleaned together

## Common Use Cases

### Regular Maintenance

```bash
# Weekly cleanup script
otto clean --keep-days 30 --keep-last 10
```

### Aggressive Space Recovery

```bash
# Free up space aggressively
otto clean --keep-days 7 --keep-last 3 --keep-failed 14
```

### Long-Term Archival

```bash
# Keep recent history
otto clean --keep-days 90 --keep-last 20
```

### Per-Project Cleanup

```bash
# Clean old project only
otto clean --project-filter old_proj --keep-days 7
```

### Audit Before Delete

```bash
# See what will be deleted
otto clean --dry-run

# Review and confirm
otto clean
```

## Performance

### Database Mode
- **Query time**: <10ms for 1,000 runs
- **Query time**: <100ms for 10,000 runs
- **Deletion time**: ~100ms per run (filesystem bound)

### Filesystem Mode
- **Scan time**: ~1s per 1,000 run directories
- **Memory usage**: Minimal (streaming scan)

## Troubleshooting

### Database Unavailable

If database is missing or corrupted:
```bash
# Use filesystem fallback
otto clean --no-db

# Database will be automatically recreated on next run
```

### Inconsistent State

If database and filesystem are out of sync:
```bash
# Verify with dry run
otto clean --dry-run

# Database will self-heal on next run
```

### Disk Space Not Freed

Check actual file deletion:
```bash
# Verify cleanup happened
ls -lh ~/.otto/otto-*/

# Check disk usage
du -sh ~/.otto
```

## Related Commands

- [`otto history`](history.md) - View runs before cleaning
- [`otto stats`](stats.md) - Understand disk usage patterns

## See Also

- [Architecture: SQLite Integration](../architecture/sqlite-integration.md)
- [Migration Guide](../migration-guide.md)
