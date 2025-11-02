# `otto stats` - Execution Statistics

The `stats` command provides aggregate metrics and analytics about Otto executions, helping you understand performance trends, success rates, and resource utilization.

## Usage

```bash
otto stats [OPTIONS]
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--task <NAME>` | Show statistics for specific task | overall stats |
| `--json` | Output in JSON format | false |

## Examples

### Overall Statistics

```bash
# System-wide statistics
otto stats
```

**Output:**
```
Overall Statistics
──────────────────────────────────────
Total Runs:              247
Successful:              234 (94.7%)
Failed:                  13 (5.3%)
Running:                 0 (0.0%)
Total Tasks:             1,482
Total Disk Usage:        12.3 GB
Total Duration:          3h 42m 15s
Average Run Duration:    54.2s
```

### Task-Specific Statistics

```bash
# Statistics for a specific task
otto stats --task build
```

**Output:**
```
Task Statistics: build
──────────────────────────────────────
Total Executions:        247
Successful:              240 (97.2%)
Failed:                  7 (2.8%)
Skipped:                 0 (0.0%)
Average Duration:        8.5s
Min Duration:            3.2s
Max Duration:            45.7s
Total Time:              35m 4s
```

### JSON Output

```bash
# Export statistics as JSON
otto stats --json

# Task-specific JSON
otto stats --task test --json
```

## Output Format

### Overall Statistics

Provides system-wide metrics across all projects and tasks:

| Metric | Description |
|--------|-------------|
| **Total Runs** | Number of Otto executions recorded |
| **Successful** | Runs that completed successfully (percentage) |
| **Failed** | Runs that failed (percentage) |
| **Running** | Currently executing runs (percentage) |
| **Total Tasks** | Aggregate number of task executions |
| **Total Disk Usage** | Combined size of all run artifacts |
| **Total Duration** | Cumulative execution time |
| **Average Run Duration** | Mean time per run |

### Task Statistics

Detailed metrics for a specific task:

| Metric | Description |
|--------|-------------|
| **Total Executions** | Times this task has run |
| **Successful** | Successful completions (percentage) |
| **Failed** | Failed executions (percentage) |
| **Skipped** | Times task was skipped (percentage) |
| **Average Duration** | Mean execution time |
| **Min Duration** | Fastest execution |
| **Max Duration** | Slowest execution |
| **Total Time** | Cumulative time spent on this task |

## JSON Output Schema

### Overall Stats

```json
{
  "total_runs": 247,
  "successful_runs": 234,
  "failed_runs": 13,
  "running_runs": 0,
  "total_tasks": 1482,
  "total_disk_usage": 13194142720,
  "total_duration_seconds": 13335.6,
  "successful_executions": 1469,
  "failed_executions": 13,
  "skipped_executions": 0
}
```

### Task Stats

```json
{
  "task_name": "build",
  "total_executions": 247,
  "successful_executions": 240,
  "failed_executions": 7,
  "skipped_executions": 0,
  "avg_duration_seconds": 8.5,
  "min_duration_seconds": 3.2,
  "max_duration_seconds": 45.7,
  "total_duration_seconds": 2104.3
}
```

## Use Cases

### Performance Monitoring

Track system performance over time:

```bash
# Get baseline metrics
otto stats --json > baseline.json

# Later, compare
otto stats --json > current.json
diff <(jq . baseline.json) <(jq . current.json)
```

### Reliability Tracking

Monitor success rates:

```bash
# Overall reliability
otto stats | grep "Successful"

# Per-task reliability
otto stats --task ci | grep "Successful"
```

### Resource Planning

Understand resource requirements:

```bash
# Check disk usage trends
otto stats --json | jq '.total_disk_usage / 1024 / 1024 / 1024 | floor'

# Average execution time
otto stats --json | jq '.total_duration_seconds / .total_runs'
```

### Identifying Slow Tasks

Find tasks that need optimization:

```bash
# Get all task stats and sort by duration
for task in $(otto --help | grep -A100 "Commands:" | tail -n+2 | awk '{print $1}'); do
  echo "$task: $(otto stats --task $task --json 2>/dev/null | jq -r '.avg_duration_seconds // 0')"
done | sort -t: -k2 -rn | head -10
```

### Capacity Planning

Estimate future resource needs:

```bash
# Disk usage per run
otto stats --json | jq '.total_disk_usage / .total_runs / 1024 / 1024 | floor'

# Runs per day
otto history --json | jq 'group_by(.timestamp / 86400 | floor) | map(length) | add / length'
```

## Metrics Explained

### Success Rate

Percentage of runs/tasks that completed without errors:
- **90-100%**: Excellent reliability
- **75-89%**: Good, but investigate failures
- **Below 75%**: Requires attention

### Average Duration

Mean execution time helps identify:
- Performance regressions (increasing over time)
- Optimization opportunities (outliers)
- Capacity planning needs

### Disk Usage

Total space consumed by Otto artifacts:
- Run directories (`~/.otto/otto-*/`)
- Task outputs, logs, and caches
- Use `otto clean` to manage

## Notes

- **Database Requirement**: Requires SQLite integration (Phase 2+)
- **Historical Data**: Statistics include all recorded runs since database initialization
- **Real-time**: Reflects current state; running tasks counted separately
- **Accuracy**: Based on recorded metadata; ensure runs complete properly for accurate stats

## Related Commands

- [`otto history`](history.md) - View detailed execution history
- [`otto clean`](clean.md) - Manage disk usage
- [`otto graph`](graph.md) - Visualize dependencies

## Performance Considerations

- Statistics queries are optimized with database indexes
- Typical query time: <50ms for 10,000+ runs
- Large datasets (100k+ runs): May take a few seconds

## See Also

- [Architecture: SQLite Integration](../architecture/sqlite-integration.md)
- [Migration Guide](../migration-guide.md)

