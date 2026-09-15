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
