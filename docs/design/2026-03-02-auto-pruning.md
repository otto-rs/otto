# Design Document: Automatic Run Pruning

**Author:** Scott Idler
**Date:** 2026-03-02
**Status:** Implemented
**Review Passes Completed:** 5/5

## Summary

Otto stores every task execution as a directory tree under `~/.otto/`, accumulating stdout/stderr logs, script copies, metadata, and artifacts forever. This design adds retention configuration to `OttoSpec` and an automatic post-run pruning system so disk usage stays bounded without manual intervention. It also addresses the unbounded `otto.log` growth.

## Problem Statement

### Background

Otto's file-based run history is a deliberate architectural choice. Each run gets an isolated `~/.otto/<name>-<hash>/<timestamp>/` directory containing per-task logs, scripts (symlinked to a content-addressed `.cache/`), outputs, and metadata. A SQLite database (`otto.db`) mirrors this data for efficient querying. This dual-tier system provides excellent debuggability and zero-dependency inspection (`cat`, `ls`) while supporting structured queries via `otto History` and `otto Stats`.

A `Clean` command already exists with solid retention semantics: `--keep-days`, `--keep-last`, `--keep-failed`, `--dry-run`, and `--project-filter`. It supports both database-backed and filesystem-fallback modes.

### Problem

Nothing ever invokes `Clean` automatically. On machines running otto frequently (CI systems, development workstations with multiple projects), `~/.otto/` grows unbounded. In practice this has caused disk-full incidents. The log file at `~/.local/share/otto/logs/otto.log` also appends forever with no rotation.

Three storage paths grow without bound:

1. **Run directories** — one per invocation, each containing logs, metadata, and artifacts. This is the primary growth vector.
2. **Log file** — `otto.log` uses `append(true)` with no size check or rotation. Currently 9.5MB on a single workstation.
3. **Script cache** — `.cache/` directories use content-addressed deduplication (good), but orphaned entries are never cleaned (minor).

### Goals

- Add retention policy configuration to `OttoSpec` so projects can declare their own cleanup preferences
- Implement automatic post-run pruning that runs opportunistically after task execution
- Add log rotation to prevent `otto.log` from growing unbounded
- Preserve the existing `Clean` command and its full CLI interface
- Zero user action required — pruning works out of the box with sensible defaults

### Non-Goals

- Changing the file-based architecture (it's the right design)
- Real-time disk space monitoring or alerts
- Remote/distributed cleanup coordination
- Pruning the SQLite database itself (WAL mode handles this adequately)
- Adding a separate `Prune` CLI command (the existing `Clean` command is sufficient)

## Proposed Solution

### Overview

Three changes, ordered by impact:

1. **Retention config** — Add a `retention` section to `OttoSpec` with fields matching the existing `CleanCommand` flags.
2. **Auto-prune after run** — After each run completes, check whether pruning is due and run it in-process using the existing cleanup logic. Throttle to at most once per 24 hours.
3. **Log rotation** — At startup, check `otto.log` size and rotate if it exceeds a threshold.

### Architecture

```
  main.rs                          app.rs
  ───────                          ──────
  setup_logging()                  execute_with_terminal_output() / execute_with_tui()
    │                                │
    ├─ rotate otto.log               ├─ Workspace::new() + init()
    │  if > 10MB                     ├─ result = scheduler.execute_all().await
    │                                ├─ record_run_complete_in_db()  [sync]
    ├─ Parser::parse()               ├─ auto_prune().await  [best-effort, runs even on failure]
    ├─ RuntimeConfig::from_parser()  │    │
    │    (includes RetentionSpec)    │    ├─ stat(.last_prune) → skip if recent
    └─ otto::run(config)            │    ├─ CleanCommand cleanup logic
                                    │    └─ touch .last_prune
                                    └─ result?  [propagate original error]
```

The auto-prune step (called from `app.rs` after task execution completes):

1. Checks mtime of `~/.otto/.last_prune` via `fs::metadata()` — single syscall
2. If fewer than `prune_interval_hours` have elapsed, returns immediately (fast path)
3. Otherwise, constructs a `CleanCommand` from the `RetentionSpec` and runs cleanup
4. Touches `.last_prune` on success (creates/overwrites the file, setting mtime to now)
5. Any errors are logged via `log::warn!` but **do not** propagate — task execution has already succeeded

```rust
// Pseudocode for src/executor/pruning.rs
pub async fn auto_prune(otto_home: &Path, retention: &RetentionSpec) -> Result<()> {
    if !retention.auto_prune {
        return Ok(());
    }

    let marker = otto_home.join(".last_prune");
    if let Ok(meta) = fs::metadata(&marker) {
        let age = meta.modified()?.elapsed()?;
        if age < Duration::from_secs(retention.prune_interval_hours * 3600) {
            return Ok(()); // Fast path: too soon
        }
    }
    // .last_prune missing or stale → prune now

    let cmd = CleanCommand {
        keep_days: retention.keep_days,
        keep_last: Some(retention.keep_last),
        keep_failed: Some(retention.keep_failed),
        dry_run: false,
        project_filter: None,
        no_db: false,
    };
    cmd.execute().await?;

    // Touch marker file
    fs::File::create(&marker)?;
    Ok(())
}
```

### Data Model

#### OttoSpec additions

```rust
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RetentionSpec {
    /// Delete runs older than this many days (default: 30)
    #[serde(default = "default_keep_days")]
    pub keep_days: u64,

    /// Always keep at least this many most recent runs (default: 10)
    #[serde(default = "default_keep_last")]
    pub keep_last: usize,

    /// Keep failed runs for this many days (default: 60)
    #[serde(default = "default_keep_failed")]
    pub keep_failed: u64,

    /// Enable automatic pruning after runs (default: true)
    #[serde(default = "default_auto_prune")]
    pub auto_prune: bool,

    /// Minimum hours between auto-prune runs (default: 24)
    #[serde(default = "default_prune_interval_hours")]
    pub prune_interval_hours: u64,
}
```

Added to `OttoSpec`:

```rust
pub struct OttoSpec {
    // ... existing fields ...
    #[serde(default)]
    pub retention: RetentionSpec,
}
```

**Note:** `max_log_bytes` is intentionally excluded from `RetentionSpec`. Log rotation happens in `main.rs:setup_logging()` before any ottofile is parsed, so it cannot depend on per-project config. Instead, log rotation uses a hardcoded 10MB threshold (or a future `OTTO_MAX_LOG_BYTES` env var).

#### Throttle file

`~/.otto/.last_prune` — an empty file whose **mtime** serves as the "last pruned" timestamp. This makes the check a single `stat()` call with no file content parsing. Updated via `File::create()` (which sets mtime to now) after a successful prune.

### API Design

#### New internal function

```rust
// In a new src/executor/pruning.rs
pub async fn auto_prune(
    otto_home: &Path,
    retention: &RetentionSpec,
) -> Result<()>
```

This is not a public CLI change. The `Clean` command remains the user-facing interface. Auto-prune reuses `CleanCommand`'s internal cleanup logic.

**Integration point:** Auto-prune is called from `app.rs` after `scheduler.execute_all().await` returns (even on failure) — not from `record_run_complete_in_db()`, which is sync and cannot call async code. The `RuntimeConfig` is extended to carry `RetentionSpec`. The `Parser` struct already stores `config_spec: ConfigSpec` (which contains `otto: OttoSpec`); we add a `pub fn retention(&self) -> RetentionSpec` accessor on `Parser` (clones the value) so `RuntimeConfig::from_parser()` can extract it.

**Cross-project scope:** Auto-prune cleans **all** projects under `~/.otto/`, not just the current one. This means if project A has `keep_days: 7` and project B has `keep_days: 90`, whichever project triggers the prune determines the retention policy applied to both. This is acceptable because:
- The throttle (default 24h) means only one project's settings apply per day
- Defaults are conservative (30 days, keep-last 10)
- Users who need per-project control use `otto Clean --project-filter` directly
- A global config override layer can be added later if this becomes a pain point

#### OttoSpec YAML interface

```yaml
otto:
  name: my-project
  retention:
    keep_days: 14
    keep_last: 5
    keep_failed: 30
    auto_prune: true
    prune_interval_hours: 12
```

All fields are optional with sensible defaults. An ottofile with no `retention:` section behaves identically to today, except auto-prune is enabled with defaults.

### Implementation Plan

#### Phase 1: RetentionSpec in OttoSpec

**Files:** `src/cfg/otto.rs`, `src/cfg/config.rs`

- Add `RetentionSpec` struct with `Default` impl and serde defaults
- Add `#[serde(default)] pub retention: RetentionSpec` field to `OttoSpec`
- Update `default_otto()` to include `retention: RetentionSpec::default()`
- Tests: deserialize ottofile YAML with and without `retention:` section; verify defaults

#### Phase 2: Thread retention config to RuntimeConfig

**Files:** `src/app.rs`, `src/cli/parser.rs`

- Add `pub fn retention(&self) -> RetentionSpec` to `Parser` (clones from `self.config_spec.otto.retention`)
- Add `retention: RetentionSpec` field to `RuntimeConfig`
- In `RuntimeConfig::from_parser()`, call `parser.retention()` to populate the field
- Update `execute_tasks()`, `execute_with_terminal_output()`, and `execute_with_tui()` signatures to accept `RetentionSpec`
- Tests: verify `RuntimeConfig` round-trips retention settings

#### Phase 3: Auto-prune logic

**Files:** new `src/executor/pruning.rs`, `src/executor/mod.rs`, `src/app.rs`

- Create `src/executor/pruning.rs` with `pub async fn auto_prune(otto_home: &Path, retention: &RetentionSpec) -> Result<()>`
- Implement throttle: `stat(.last_prune)` → check mtime age → skip or proceed
- Construct `CleanCommand` from `RetentionSpec` fields and call `execute()` (the struct is already fully public)
- In `app.rs`, auto-prune must run **regardless of whether tasks succeeded or failed**. The current pattern uses `scheduler.execute_all().await?` which returns early on failure. Restructure to:
  ```rust
  let result = scheduler.execute_all().await;
  // Auto-prune runs even if tasks failed — failing CI jobs that never prune
  // are exactly the scenario that fills disks
  if let Err(e) = pruning::auto_prune(&otto_home, &config.retention).await {
      log::warn!("Auto-prune failed: {}", e);
  }
  result?; // Propagate the original error after pruning
  ```
- Apply this pattern in both `execute_with_terminal_output()` and `execute_with_tui()`
- `otto_home` is derived the same way as `Workspace`: `$OTTO_HOME` or `$HOME/.otto` — extract a shared `fn resolve_otto_home() -> PathBuf` helper
- **Suppress output:** `CleanCommand` currently prints to stdout via `println!`. During auto-prune, this would be noisy/confusing. Either: (a) add a `quiet: bool` field to `CleanCommand` that suppresses `println!` calls, or (b) extract the core cleanup logic into a function that returns results without printing, and have both `CleanCommand::execute()` and `auto_prune()` call it. Option (b) is cleaner but more refactoring; option (a) is pragmatic.
- Tests: tempdir-based tests for throttle skip, throttle expire, prune execution, error handling

#### Phase 4: Log rotation

**Files:** `src/main.rs`

- In `setup_logging()`, before `OpenOptions::new().append(true)`:
  - Check `log_file_path.metadata().map(|m| m.len())`
  - If > 10MB (`10 * 1024 * 1024`), rename to `otto.log.1` then proceed
- One backup file maximum — simple and predictable
- Hardcoded threshold (runs before ottofile parsing, no config available)
- Optionally respect `OTTO_MAX_LOG_BYTES` env var for override
- Tests: create oversized file, verify rotation occurs

#### Phase 5: Cache pruning (optional, low priority)

**Files:** `src/executor/pruning.rs`

- Walk `~/.otto/otto-*/.cache/<task>/` directories
- For each cached script hash file, scan sibling timestamp dirs for symlinks pointing to it
- Remove orphaned cache entries with no remaining references
- Only run during auto-prune, gated behind a `should_prune_cache()` check
- This phase can be deferred indefinitely — cache growth is slow due to content-addressed deduplication

## Alternatives Considered

### Alternative 1: External cron job

- **Description:** Document a cron/systemd-timer that runs `otto Clean` periodically
- **Pros:** Zero code changes, works immediately
- **Cons:** Requires per-machine setup, easy to forget, CI environments rarely have user crontabs, doesn't help the user who just hit a full disk
- **Why not chosen:** The whole point is that users shouldn't have to think about this

### Alternative 2: Global config file at `~/.otto/config.yaml`

- **Description:** Store retention settings in a global config separate from ottofiles
- **Pros:** Single source of truth across all projects
- **Cons:** Adds another config file to discover and maintain, projects can't customize their own retention
- **Why not chosen:** Putting it in `OttoSpec` means it's co-located with the project definition and inherits otto's existing config resolution. A global config could be added later as an override layer if needed.

### Alternative 3: Max total size instead of age-based retention

- **Description:** Set a cap like `max_size: 1GB` and prune oldest runs when exceeded
- **Pros:** Directly addresses the disk space concern
- **Cons:** Requires walking and summing directory sizes on every check (expensive), hard to predict which runs get deleted, less intuitive for users
- **Why not chosen:** Age-based retention is simpler, predictable, and the existing `CleanCommand` already implements it well. Size-based could be added as an optional secondary constraint later.

### Alternative 4: Replace file-based storage with database-only

- **Description:** Stop writing run directories entirely, store everything in SQLite
- **Pros:** Single growth vector, easier to manage
- **Cons:** Loses zero-dependency debuggability, loses filesystem isolation benefits, large stdout/stderr blobs in SQLite perform poorly, major architectural rewrite
- **Why not chosen:** The file-based design is genuinely good for this use case. The problem isn't the architecture — it's the missing janitor.

## Technical Considerations

### Dependencies

- No new external dependencies required
- Uses existing `CleanCommand` logic, `OttoSpec` serde machinery, and filesystem operations
- Log rotation uses only `std::fs` operations

### Performance

- **Fast path:** Auto-prune check is a single `stat()` on `.last_prune` mtime — negligible overhead on every run
- **Slow path:** Actual pruning only runs once per 24h (configurable). The existing `CleanCommand` filesystem scan is O(n) in total runs — acceptable since it only runs infrequently
- **Log rotation:** Single file size check at startup — O(1)

### Security

- No new attack surface. All operations are local filesystem.
- `.last_prune` uses mtime (no content to corrupt)
- Retention defaults are conservative (30 days, keep-last 10) to avoid accidentally deleting data users still want

### Testing Strategy

- **Unit tests:** `RetentionSpec` deserialization, default values, throttle logic
- **Integration tests:** Auto-prune with tempdir (replicating existing `CleanCommand` test patterns)
- **MemFs tests:** Throttle file read/write, prune orchestration
- **Existing test preservation:** All current `CleanCommand` tests continue to pass unchanged

### Rollout Plan

- Ship as a minor version bump (non-breaking — all new fields have defaults)
- Auto-prune is on by default but can be disabled with `auto_prune: false`
- Existing `otto Clean` CLI behavior is unchanged
- No migration needed — `.last_prune` is created on first auto-prune run

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Auto-prune slows down task execution | Low | Low | Throttle to 1x/24h; prune is fast for reasonable history sizes |
| Prune deletes data user still wants | Low | Medium | Conservative defaults (30 days, keep-last 10); `--dry-run` for manual checks |
| `.last_prune` stat fails | Low | Low | Treat as "never pruned" and prune now (safe to over-prune) |
| Race condition: two otto processes prune simultaneously | Low | Low | Both would try to delete the same dirs; `remove_dir_all` on already-deleted dir just returns error, which is already handled gracefully |
| Log rotation loses data mid-write | Low | Low | Rotation happens at startup before any logging; single rename operation |
| `keep_last` applies globally, not per-project | Medium | Low | The existing `CleanCommand` behavior; users who need per-project granularity use `--project-filter`. Document this clearly. |
| Multi-user environments (CI with shared `~/.otto/`) | Low | Low | Pre-existing behavior in `CleanCommand`; file permissions prevent cross-user deletion. Not a new concern. |
| Disk already full when auto-prune tries to touch `.last_prune` | Low | Low | Touch happens after deletions free space; if it still fails, we simply prune again next time |

## Open Questions

- [ ] Should there be a global `~/.otto/config.yaml` that overrides per-project retention settings? (Deferred — can be added later without breaking changes)
- [x] Should auto-prune only clean the current project's runs, or all projects? **Decision: all projects** — the throttle ensures it's infrequent, and the whole point is bounding total `~/.otto/` growth
- [x] Should `otto.db` get a `VACUUM` during auto-prune? **Decision: no** — SQLite WAL mode handles space reclamation automatically, and VACUUM requires an exclusive lock
- [ ] Should `CleanCommand` get a `quiet` mode, or should we extract the core logic into a separate function? (Decide during Phase 3 implementation based on refactoring effort)

## References

- Existing implementation: `src/cli/commands/clean.rs`
- Workspace storage: `src/executor/workspace.rs`
- Config model: `src/cfg/otto.rs`
- State management: `src/executor/state/manager.rs`
- Log setup: `src/main.rs:9-24`
