# Implementation Notes: Idle Task Heartbeat

Running, append-only record of how the implementation of
`docs/design/2026-09-15-idle-task-heartbeat.md` diverges from or interprets the
design doc. Appended once per phase at commit-prep time. A later entry
supersedes an earlier one rather than rewriting it.

## Phase 0: Measure what heartbeat lines break in the existing suite

No production code, no commit. Full result table is recorded in the design doc
under Phase 0; only the decisions are here.

### Design decisions
- Probe placed in `main()` immediately after `info!("Starting otto")`
  (`src/main.rs:188`) rather than in the scheduler. The doc did not say where.
  Chosen because the point of the probe is maximum exposure: an ungated line for
  every invocation, so the measurement over-reports rather than under-reports.
- Probe presence verified in the built binary (`strings target/debug/otto`) and
  firing verified on a live 30s task (30 lines) before trusting the suite
  result. A green run from a probe that never fired would be the worst possible
  Phase 0 output.

### Deviations
- **The doc specified a 1-second probe; the load-bearing measurement was taken
  at 100ms firing immediately.** At 1s the suite was green across 49 suites, but
  `cli_surface_test` (14 tests in 0.05s) and `tasks_flag_test` (8 tests in
  0.04s) never saw a single probe line, so that green was vacuous for exactly
  the suites the doc named as the risk. Both results are recorded; the 1s figure
  is reported as the doc asked and then explained away.
- Added `--no-fail-fast`. The first aggressive run exited at the first failing
  test binary with 16 suites unrun, which would have produced a second
  incomplete failure set.
- Added a fourth task (`crbar`, a `\r`-redrawn bar with no newline) to the
  three-task probe ottofile the doc specifies. It is the motivating
  `philo slim-dump` shape and nothing in the doc's three tasks exercises it.

### Tradeoffs
- Probe in `main()` vs. in the scheduler. `main()` over-reports: it produced two
  failures that the shipped ticker cannot cause, because both invocations
  (`otto makefile convert`, `otto --tasks`) execute no task. Taken deliberately:
  a scheduler-placed probe would have reported zero and taught nothing, whereas
  the over-report converted into a Phase 3 constraint (the ticker must not be
  armed for a non-executing invocation) plus the two existing tests that
  enforce it.

### Open questions
- None.

## Phase 1: Flag, env var, ottofile key

`--progress-interval` (env `OTTO_PROGRESS_INTERVAL`, default `10`) and
`otto.progress-interval` are wired end to end: `Parser::global_args()` ->
`Parser::parse()`'s `value_source` precedence (flag > env > ottofile >
default, copying `jobs`) -> `RunPlan::progress_interval` ->
`RuntimeConfig::progress_interval` -> `TaskScheduler::set_progress_interval()`.
Nothing reads the field yet; that starts in Phase 2 (the idle clock) and
Phase 3 (the ticker).

### Design decisions
- `TaskScheduler` gained `progress_interval` as a private field plus a
  `set_progress_interval()` setter, the same shape as the existing
  `no_prefix`/`set_no_prefix()` pair (`src/executor/scheduler.rs`), rather than
  a `TaskScheduler::new()` constructor parameter. `no_prefix`'s own doc comment
  gives the reason this was copied for: the many existing `TaskScheduler::new()`
  call sites (tests included) don't have to thread a flag almost none of them
  exercise.
- `progress_interval` is threaded into `execute_tasks()` and
  `execute_with_terminal_output()` but deliberately **not** into
  `execute_with_tui()` (`src/app.rs`). The design doc's Non-Goals section
  excludes TUI entirely ("`--tui` owns its rendering") and Phase 3 suppresses
  emission under `tui_mode` outright, so there is nothing for the TUI path to
  receive yet. This mirrors `no_prefix`, which is likewise never threaded into
  `execute_with_tui()` today.
- `OttoSpec.progress_interval` is `Option<u64>` with
  `#[serde(default, rename = "progress-interval", skip_serializing_if =
  "Option::is_none")]`, matching `envs_command`'s shape exactly (kebab rename,
  no custom deserializer) rather than `jobs`'s shape (which carries a
  `deserialize_with` guard rejecting `0`). `0` is a legal, meaningful value
  here ("disables"), so there is no equivalent hot-spin hazard to guard
  against and no custom deserializer was added.

### Deviations
- **`docs/commands/ottofile-reference.md`'s drift test
  (`ottofile_reference_key_inventory_is_exhaustive`,
  `src/cfg/task_tests.rs`) required updating independent of anything the
  design doc's Phase 1 section named.** Adding a ninth `OttoSpec` field bumped
  its on-disk key count from 8 to 9 and the reference doc's total from 46 to
  47; both the destructuring compile-time trigger and the doc's table/prose
  needed the new key. Same effect as the doc's ask (thread the key through
  cleanly), different seam (a pre-existing cross-file consistency test the
  design doc's Phase 1 bullets did not mention) than the doc's stated success
  criteria named.
- **The rendered `--help` order is `[env: ...]` then `[default: ...]`,**
  confirmed by running the golden-snapshot test rather than by trusting the
  design doc's own API Design section, which rendered `[default: 10] [env:
  OTTO_PROGRESS_INTERVAL=]` (default first). clap's actual rendering order
  disagreed; the pinned snapshot (`EXPECTED_GLOBAL_OPTIONS_HELP_TEMPLATE`,
  `src/cli/parser_tests_b.rs`) follows the real output, not the doc's
  illustration.
- **No test exercises `OTTO_PROGRESS_INTERVAL` by mutating the process
  environment.** A first attempt at one did, and it polluted
  `test_help_global_flags_no_drift`, which renders `--help` and therefore
  echoes whatever value the env var currently holds
  (`[env: OTTO_PROGRESS_INTERVAL=<value>]`) - any test in the same binary that
  renders help is exposed to a concurrently-running env mutation, which
  `#[serial]`-ing only the progress-interval tests against each other does not
  fix. Removed rather than serialized against the whole binary. The env
  wiring itself is still evidence-backed: the golden help snapshot proves
  clap treats `--progress-interval` as env-sourced (`[env:
  OTTO_PROGRESS_INTERVAL=]` renders only for an `Arg` built with `.env(...)`),
  and flag/ottofile precedence around it is covered by the tests that remain.

### Tradeoffs
- `TaskScheduler`'s `progress_interval` field defaults to a literal `10` in
  `TaskScheduler::new()` rather than referencing
  `cli::parser::DEFAULT_PROGRESS_INTERVAL`, duplicating the magic number
  across the `cli` and `executor` modules. Reusing the constant across module
  boundaries did not fit today's layering (`cli` depends on `executor`, not
  the reverse), and the duplication is inert in practice: every production
  call site sets the field explicitly via `set_progress_interval()`
  immediately after construction, so the field's own initializer never
  reaches a real run.

### Open questions
- None.

## Phase 2: The ledger and the idle clock

The per-task idle clock exists and is stamped; nothing is printed. A new
`src/executor/heartbeat.rs` holds `TaskClock` (the doc's Data Model struct) and
`TaskClocks` (the per-run map plus the single run-start `Instant` its offsets
are measured from). `TeeWriter::write` stamps the clock at its top, above the
`suppress_terminal` branch. Liveness is not duplicated: the live-child registry
moved from `execute_all`'s `ActiveTasks` local onto `TaskScheduler`, and a tick
reads that map itself.

### Design decisions
- **`last_line_ms` is stamped at the top of `TeeWriter::write`
  (`src/executor/output.rs`), above the `suppress_terminal` branch.** Verified
  by breaking it: moving the stamp inside the branch, beside `terminal_lock()`,
  turns `a_suppressed_subtasks_line_advances_its_own_clock_and_no_siblings` red
  with `a suppressed subtask's line must still reset its clock: 0 vs 0`, which
  is the buffered-foreach case the doc's placement argument is about.
- **`TaskClock` and `TaskClocks` live in a new module rather than in `output.rs`
  or `scheduler.rs`.** Both read it - the drains stamp it, the scheduler
  intersects it with the registry - so it belongs to neither, and Phase 3's
  ticker is a third reader.
- **The live-child registry is now owned by `TaskScheduler`
  (`live_children`), handed to `ActiveTasks::with_children`.** It was a private
  field of a local in `execute_all`, unreachable from `&self`, so neither a
  tick nor a test could read it without a mirror - which is the one thing the
  design forbids. Nothing about the registry's own lifecycle changed:
  `register_child`, `deregister_child` and `abort_all` still write the same map.
- **A `tty: true` task is excluded by absence, not by a predicate.** It returns
  before any `TaskStreams` exist (`task_execution.rs`), so `clocks.start()` is
  never reached for it, so the intersection in `TaskClocks::candidates` cannot
  name it. Pinned by `a_live_task_with_no_clock_is_not_a_candidate`. No tick-time
  code reads `task.tty`.
- **`TaskClocks::candidates` returns names in sorted order**, so a tick with
  several lines to emit emits them in a stable order rather than a hash order
  that changes between beats.
- **The clock map recovers from lock poisoning** the same way `terminal_lock`
  does, with the same reasoning: it guards no invariant, and refusing to time
  anything for the rest of the run is the worse failure.
- **Everything new is `pub` in a `pub` module**, so the three accessors Phase 3
  is the first production reader of (`note_beat`, `since_beat_ms`, `elapsed`)
  need no `#[allow(dead_code)]`. Each has a test here; `-D warnings` is clean
  with no lint allowances added anywhere.

### Deviations
- **`TaskClock` carries a fourth field the doc's struct does not show:
  `origin: Instant`, a copy of `TaskClocks::origin`.** Same model as the doc
  specifies (both ms fields are offsets from one run-start `Instant`); the copy
  is what lets `note_line` stamp without taking the map's lock, on a path that
  runs once per line of task output. Same effect, one field wider.
- **Both ms fields start at the clock's own creation offset, not at 0.** A task
  is silent from the moment its clock starts; zeroing them would report a task
  that began late in a long run as having been silent since the run started.
- **The clock is created in the task body immediately after its `TaskStreams`
  are obtained, not inside `TaskStreams::new`.** The doc says "created when a
  task's `TaskStreams` are created", and on the run path this is that instant -
  but `TaskStreams::new` is also called from the TUI pre-creation path
  (`app.rs`), which has no run-scoped map to insert into and runs for tasks that
  may never spawn a child. Same seam for the intent, correct seam for the
  lifecycle: a clock now exists exactly when a non-`tty:` child does.
- **The SIGINT half of the second success criterion is driven through
  `CancelSignal` in-process, not by delivering a real SIGINT.** The candidate
  set is only observable from inside the process, and a real signal would need
  a subprocess that cannot be asked. The code path under test is identical:
  `install_stop_handler` (`app.rs`) does nothing but trip this signal, and the
  assertion covers `abandon_run` -> `abort_all` exactly as the doc names it.
- **New sibling test file `src/executor/scheduler_tests_c.rs`.** Appending the
  run-level tests to `scheduler_tests_b.rs` put it at 1568 lines, past the
  1500-line Rust cap the existing `tests_a`/`tests_b` split exists to respect.
- **Phase 1's `progress_interval` is still unread.** Phase 2 adds no reader for
  it, exactly as the doc's phase split has it; the field's comment still points
  at Phase 3.

### Tradeoffs
- **`tick_candidates()` is async here; Phase 3's ticker needs a blocking read.**
  The registry is a `tokio::sync::Mutex`, and this phase's only callers are
  async (the tests and, later, nothing). The intersection itself lives in
  `TaskClocks::candidates`, which is lock-agnostic, so the ticker thread takes
  `blocking_lock` on the registry and calls the same function rather than
  carrying a second copy of the logic.
- **A clock is never removed, only stopped being read.** The alternative is
  deregistering it beside `deregister_child`, which would add a sixth update
  site to a design whose whole argument is about not having a set of update
  sites to get wrong. A stale entry is unreachable without a live child, and
  `a_cancelled_run_leaves_no_candidates` asserts the entry survives a
  cancellation while the candidate set is empty - so the empty read is provably
  coming from the liveness source of truth.
- **`TeeWriter` holds an `Arc<TaskClock>` rather than looking its clock up by
  name.** One pointer per drain and no lock per line, against a map lock taken
  once per line of every task's output.
- **The run-level tests observe through a sampler rather than a hook.** The two
  criteria about counts are about what a tick would see at an arbitrary moment,
  so a sampler is the honest instrument - but every state it samples is held by
  a fifo until the test releases it, so a regression that started a third child
  would hold that reading rather than flash past. `assert_eq!(max, 2)` carries
  both halves of the criterion (reaches 2, never exceeds 2) as one claim.

### Open questions
- None.
