# `otto history` - View Execution History

The `history` command displays a chronological record of all Otto runs, providing insights into execution patterns, success rates, and resource usage.

## Usage

```bash
otto history [OPTIONS]
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--limit <N>` | Maximum number of runs to display | 20 |
| `--status <STATUS>` | Filter by status (success, failed, running) | all |
| `--project <HASH>` | Filter by project hash | all projects |
| `--task <NAME>` | Show history for specific task only | all tasks |
| `--json` | Output in JSON format | false |

## Examples

### View Recent Runs

```bash
# Show last 20 runs (default)
otto history

# Show last 50 runs
otto history --limit 50
```

### Filter by Status

```bash
# Show only successful runs
otto history --status success

# Show only failed runs
otto history --status failed

# Show currently running executions
otto history --status running
```

### Filter by Project

```bash
# Show history for specific project
otto history --project abc123
```

### Task-Specific History

```bash
# Show history for a specific task
otto history --task build

# Last 10 executions of the test task
otto history --task test --limit 10
```

### JSON Output

```bash
# Export history as JSON for scripting
otto history --json | jq '.[] | select(.status == "failed")'
```

## Output Format

### Run History Table

```
Timestamp            Status  Duration  Size      User     Path
────────────────────────────────────────────────────────────────
2025-11-02 14:23:45  ✓       12.3s     45.2 MB   saidler  ~/repos/myproject
2025-11-02 13:15:22  ✗       8.5s      32.1 MB   saidler  ~/repos/myproject
2025-11-02 11:42:10  ✓       15.8s     52.3 MB   saidler  ~/repos/myproject
```

**Columns:**
- **Timestamp**: When the run started
- **Status**:
  - `✓` (green) - Successful completion
  - `✗` (red) - Failed execution
  - `⋯` (yellow) - Still running
- **Duration**: Total execution time (or `-` if still running)
- **Size**: Disk space used by run artifacts
- **User**: Username who initiated the run
- **Path**: Working directory where Otto was executed

### Task History Table

When viewing task-specific history (`--task <name>`):

```
Timestamp            Status     Exit Code  Duration  Path
─────────────────────────────────────────────────────────────
2025-11-02 14:23:45  Completed  0          5.2s      ~/repos/myproject
2025-11-02 13:15:22  Failed     1          3.1s      ~/repos/myproject
2025-11-02 11:42:10  Completed  0          6.8s      ~/repos/myproject
```

**Additional Columns:**
- **Exit Code**: Process exit code (0 = success)
- **Status**: Completed, Failed, Skipped, Running

## JSON Output Schema

```json
[
  {
    "id": 123,
    "project_id": 45,
    "timestamp": 1730561025,
    "status": "success",
    "duration_seconds": 12.3,
    "size_bytes": 47456256,
    "ottofile_path": "/home/user/project/otto.yml",
    "cwd": "/home/user/project",
    "user": "saidler",
    "hostname": "workstation",
    "args": ["build", "test"],
    "ended_at": 1730561037
  }
]
```

## Use Cases

### Debugging Failures

Find recent failures to investigate:

```bash
otto history --status failed --limit 5
```

### Performance Analysis

Track how execution time changes:

```bash
otto history --task build --limit 30 --json | jq '.[].duration_seconds'
```

### Disk Usage Monitoring

Find runs consuming excessive space:

```bash
otto history --json | jq 'sort_by(.size_bytes) | reverse | .[0:5]'
```

### Success Rate Tracking

Check success rate over time:

```bash
# All runs
otto history --json | jq 'group_by(.status) | map({status: .[0].status, count: length})'
```

## Notes

- **Database Requirement**: The `history` command requires SQLite integration (automatic since Phase 2)
- **Data Collection**: History is automatically recorded starting from when the database was initialized
- **Historical Runs**: Runs executed before database initialization won't appear unless imported
- **Real-time Updates**: Running executions show current status; refresh to see updates

## Related Commands

- [`otto stats`](stats.md) - Aggregate statistics and metrics
- [`otto clean`](clean.md) - Clean up old runs
- [`otto graph`](graph.md) - Visualize task dependencies

## See Also

- [Architecture: SQLite Integration](../architecture/sqlite-integration.md)
- [Migration Guide](../migration-guide.md)

