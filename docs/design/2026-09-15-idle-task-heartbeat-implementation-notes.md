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

## Phase 3: The ticker and the line

The heartbeat prints. `src/executor/heartbeat.rs` gained the ticker (`spawn`,
`Heartbeat`, `Shutdown`) plus the three pieces a tick is built from
(`due_beats`, `beat_line`, `emit`); `TaskScheduler::start_heartbeat()`
(`src/executor/scheduler.rs`) is its only arming site, called from
`execute_with_terminal_output` (`src/app.rs`) immediately before `execute_all`
and stopped immediately after it returns. Observed on a 25s silent task at
`--progress-interval 5`: four lines, the first 5.99s after the task's own line
and the rest 5.00s / 5.00s / 5.00s apart, `[quiet] still running (6.0s)` and up.

### Design decisions
- **The emit function lives in `heartbeat.rs`, not `replay.rs`.** See
  Deviations; it is the one placement this phase moved.
- **The ticker sleeps on a `Condvar`, not on `thread::sleep`.** The doc only
  specifies the wake cadence (`min(interval, 1s)`), and a plain sleep loop
  satisfies it - but `stop()` then has to wait out the remaining granularity,
  which holds otto's exit for up to a second after the final status line, on
  every run. The condvar keeps the cadence and makes stopping immediate.
  `stopping_is_prompt_and_the_ticker_stops_looking` pins it at under 500ms.
- **`stop()` joins the thread**, which is what turns "no heartbeat follows the
  final status line" from a race into a guarantee: a tick already inside `emit`
  holds `TERMINAL_LOCK` and finishes its write before the join returns. `Drop`
  calls the same idempotent `stop()` as a backstop for an early return or an
  unwind, so a run cannot leave a ticker beating.
- **`is_terminal()` on stderr is read once, in `spawn`, and never re-derived** -
  the npm bug the doc cites (commit `5b858c6`) is a predicate evaluated twice
  whose readings diverged. Nothing about a run can change the answer.
- **The tick's liveness read is a closure the scheduler supplies**, rather than
  `heartbeat.rs` importing `LiveChildren` and taking the lock itself. The
  registry is a `tokio::sync::Mutex` the scheduler owns, `blocking_lock` is
  legal only off-runtime, and injecting the read is what lets the whole ticker
  be unit-tested against a fake live set with no children and no runtime.
- **One `terminal_lock()` per tick, not per line.** The doc says "exactly once
  per emit"; taking it for the whole batch also keeps a tick's lines contiguous,
  which matters the moment two tasks stall at once.
- **`note_beat()` is stamped after the write, not before.** A tick can wait an
  arbitrarily long replay for the terminal lock (`write_replay_blocks` holds it
  for a whole batch), and spacing is meant to be measured from the line the user
  actually saw.
- **TUI suppression is expressed as an interval of zero** rather than a second
  predicate, so `--tui` and `--progress-interval 0` reach the same single
  "start no thread" path. Note `execute_with_tui` is never handed
  `progress_interval` at all (Phase 1), so the guard is belt-and-braces.
- **`--progress-interval 0` starts no thread**, rather than starting one that
  declines to print. Pinned by `a_zero_interval_starts_no_thread`.

### Deviations
- **The synchronous emit function is in `src/executor/heartbeat.rs`, not
  alongside `report_status_line` in `src/executor/scheduler/replay.rs` as the
  doc's Architecture table says.** Same effect, correct seam: everything the doc
  asks for about it holds (synchronous, takes `terminal_lock()` itself exactly
  once, calls nothing that takes it again, never touches `report_status_line`),
  but `replay.rs` is `include!`d into `scheduler.rs` as one impl block for
  ordered buffered-foreach replay, and the emit path shares nothing with it
  except the lock - which is `pub(crate)` in `output.rs` and reachable from
  anywhere in the crate. `heartbeat.rs` is this feature's module and already had
  the clock the emit path reads and the test file its tests belong in.
- **`status_label` was refactored rather than reused.** The doc says "Label from
  `status_label` when stderr is a terminal, and the plain `[name]` form
  otherwise", which as written duplicates `status_label`'s `--no-prefix` branch
  in a second place. Instead `colors.rs` gained `task_label(name, no_prefix)`
  (what `status_label` now delegates to, so the two cannot drift) and
  `plain_task_label(name, no_prefix)` (the same shapes with no colour, whatever
  `SHOULD_COLORIZE` says). `the_label_form_follows_stderr_not_stdout` asserts
  the heartbeat picks between exactly those two rather than inventing a third.
- **`wake_granularity()` was extracted** so `min(interval, 1s)` is a named
  function with its own test rather than an expression inside a spawn.
- **Criterion 1 and criterion 6's second half are measured on a merged stream**
  (`sh -c 'otto ... 2>&1'`, one pipe), not on two pipes. "No heartbeat appears
  after the final status line" is a claim about the relative order of a stderr
  line and a stdout line, and two pipes read by two threads only give that order
  to within scheduling jitter; one pipe makes it byte order. Criterion 2 still
  uses genuine file redirection (`Stdio::from(File)`), and criterion 3 genuine
  `script` + `2> file`, because those criteria are about the capture route
  itself.
- **One pinned figure is loosened, on the lower bound only, by 50ms**
  (`SKEW`, `tests/progress_heartbeat_test.rs`). otto measures silence from the
  moment `TeeWriter::write` stamps the clock; the test measures it from the
  moment that line reaches the far side of a pipe, which is strictly later, so a
  gap otto computed as exactly 5.000s can read as 4.999s here. That is
  measurement skew, not a beat arriving early. The upper bounds (7s for the
  first beat and for each gap), the "at least 3 lines" floor, the zero-line
  counts and every byte count are exactly as the doc pins them.
- **No timing was shortened.** All six criterion runs use the doc's 25-second
  task and 5-second interval. They are six tests in one binary, so the harness
  runs them concurrently and the suite grows by about one task's duration:
  measured at 25.30s for the whole `progress_heartbeat_test` binary.
- **Two tests beyond the six criteria.** `the_fixture_ottofile_declares_the_three_tasks`
  guards the negative criteria against a fixture the binary cannot parse (a
  broken ottofile makes "zero `still running` lines" pass for the wrong reason),
  and `the_ticker_beats_a_silent_live_task` covers the thread path in-process.
- **No new test for "the ticker is not armed for a non-executing invocation."**
  The doc's Phase 0 result says the two existing canaries are the test for it;
  both were re-run and are green
  (`makefile_converter_test::test_strict_passes_a_makefile_that_converts_cleanly`,
  `tasks_flag_test::tasks_defaults_to_yaml_on_a_real_tty`).

### Tradeoffs
- **Arming in `execute_with_terminal_output` vs. inside `execute_all`.** Inside
  would cover the scheduler's own test call sites and the TUI path for free.
  Chosen against: `execute_all` has several returns, the doc's lifetime is "a
  flag set after `execute_all` returns", and the arming site is exactly the
  constraint Phase 0 measured - one call site, as late as possible, is what
  makes "never for a non-executing invocation" reviewable by reading.
- **`the_ticker_beats_a_silent_live_task` writes one heartbeat line into the
  test binary's own stderr**, which the harness does not capture (`io::stderr()`
  bypasses the thread-local redirect `print!` uses), so `cargo test` output
  carries a stray `[quiet] still running (1.0s)`. Accepted: the alternative is
  to test `due_beats` and `beat_line` only and never exercise the thread, and
  the beat stamp it asserts on moves only after `emit` has run - so that one
  line is the proof.
- **`Vec<String>` from the liveness closure, allocated per wake.** Once a second
  at most, against a `tokio::sync::Mutex` guard that would otherwise have to be
  held across the intersection. The registry's critical sections are two lines
  long everywhere else in otto and this keeps them that way.

### Open questions
- None.

## Phase 4: Docs and coverage
### Design decisions
- **README gained a global-flags table** (`README.md`, Usage section), not just
  a `--progress-interval` row spliced into prose. No such table existed before
  this phase; the flag needed one row and the nearest existing anchor was the
  inline `bash` usage block above it, which already names `-j/--jobs`, `-t/--tui`,
  and `--no-prefix`. The table lists exactly those plus `-C/--cwd`, `-o/--ottofile`
  (both named in the prose one paragraph up) and `--progress-interval`, so the row
  this phase needs sits among the flags a reader has already seen, rather than
  alone.
- **`examples/hello-world/otto.yml` is the annotated example**, not one of the
  twenty others. It is "the smallest possible ottofile" per `examples/README.md`
  and already carries an illustrative `jobs: 16` unrelated to its three-task
  body, so a second illustrative-only key follows an existing precedent in the
  same file rather than establishing a new one.
- **Two stale comments were corrected for accuracy**, not left as drift:
  `docs/commands/ottofile-reference.md`'s `otto.progress-interval` row dropped
  "nothing reads the resolved value yet", and the doc-comment above the
  `progress-interval` `Arg` in `src/cli/parser/help.rs` dropped the matching
  "only in this phase" language, both written during Phase 1 before Phases 2-3
  wired the ticker. Neither edit changes behaviour; both are comment/prose-only.

### Deviations
- None from the doc's own Phase 4 bullets. The two stale-comment corrections
  above are additional to what Phase 4 lists, not a deviation from it: the doc
  never specified their wording, and leaving a false "nothing reads this yet"
  statement standing next to newly-shipped behaviour would be a docs regression
  this phase exists to avoid.

### Tradeoffs
- **Corrected the stale comments here rather than leaving them** for a
  hypothetical later pass. Against: strictly, Phase 4's bullets name only a
  README row and an example-ottofile line. For: both stale spots are read
  before this file (`ottofile-reference.md`) and describe this feature's own
  final state, an inaccuracy created by this feature's own earlier phases; a
  "docs and coverage" phase is the natural place to close it, and the fix is a
  two-line comment/prose diff with zero behavioural risk.

### Open questions
- None.
