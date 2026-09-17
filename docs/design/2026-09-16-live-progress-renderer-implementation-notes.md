# Implementation Notes: Live Progress Renderer

Companion to `2026-09-16-live-progress-renderer.md`. Append-only, one section
per phase, written at the end of that phase. Never edit an earlier section: a
later phase that changes an earlier decision records the change in its own
section.

## Phase 0: PTY spike

Zero production code. Two new test files, both instruments:

- `tests/progress_spike_pty_test.rs`: otto's terminal behaviour today, measured
  under a pty.
- `tests/progress_spike_indicatif_test.rs`: what indicatif 0.18.3 will and will
  not do for a stream that is not a terminal.

### Verdict: `--progress always` is CUT. The flag ships `auto | never`.

The design's claim about the draw path is CORRECT and is now pinned by a test:
`ProgressDrawTarget::term_like` draws through a non-terminal sink, because the
`TargetKind::TermLike` branch (`draw_target.rs:198`) carries no terminal
predicate, unlike `TargetKind::Term` at `:176`. `grep -c 'is_term\|is_terminal'
term_like.rs` is 0, as stated. Both halves verified.

`always` is cut on the OTHER half of the design's own condition, "otto can
supply correct `width`, `height`, and cursor operations for a stream that is not
a terminal", which three measurements say it cannot:

- **No size.** `TIOCGWINSZ` on a non-terminal fd returns -1. console reports
  that honestly (`size_checked()` gives `None`) and then invents `(24, 80)` from
  `size()`. otto would be truncating rows to `width - 1` against a number it
  made up, which is the wrap-miscount drift the Risks table already lists, with
  no mitigation available on this path.
- **No cursor.** A cursor op into a pipe is not an op, it is more bytes. 12
  redraws of ONE row wrote 181 bytes for the first frame and 2413 total, with 26
  copies of the row text. Nothing is ever erased; the log grows for the length
  of the run.
- **No throttle.** `ProgressDrawTarget::term_like` installs `rate_limiter: None`
  (`draw_target.rs:84-93`); `term()` and `stderr()` both take a refresh rate and
  `term_like` silently does not. 100 ticks of one row: 101 frames through
  `term_like`, 20 through `term_like_with_hz(_, 1)`. The design names
  `term_like` specifically and never mentions this.

### Verdict: stay on indicatif 0.18.3. otto owns the `TERM=dumb` policy.

- 0.18.6 is a two-crate bump, not one: it requires `console >= 0.16.4`
  (`indicatif-0.18.6/Cargo.toml:142-143`) against the locked 0.16.2. `is_dumb`
  does not exist in console 0.16.2 at all (`grep -rn is_dumb console-0.16.2/src`
  is empty), so "bump indicatif for `is_dumb`" also moves console.
- What the bump buys is narrower than the design implies. 0.18.6 applies
  `is_dumb()` in exactly one place, `ProgressDrawTarget::term()`
  (`draw_target.rs:80`, `if !term.is_term() || is_dumb() { return hidden() }`).
  No other path gains a check.
- otto has to own the predicate regardless: `auto` is `is_terminal()` AND `TERM
  != dumb` AND no `CI`, and no version of indicatif knows about `CI`. Taking the
  bump leaves two sources of truth for whether to draw, which is the npm
  `5b858c6` divergence the design already cites. One `TERM` read beside the `CI`
  read is one place.
- Recorded against staying, for Phase 5 rather than against this verdict: on
  0.18.3 `is_stderr()` returns `false` for `TargetKind::Multi`
  (`draw_target.rs:133-140`); 0.18.6 fixes it (`:150`). `ProgressBar::set_style`
  consults it (`progress_bar.rs:161`), so every row inside a `MultiProgress` is
  styled with console's STDOUT colour decision even though the region is on
  stderr. That is the same defect class otto fixed at `output.rs:192-194` in
  `47b4605`. **Phase 5 must set row styles explicitly and must not rely on
  indicatif's auto-targeting.**
- The verdict is enforced, not just written down. `Cargo.toml` says `indicatif =
  "0.18"`, so only `Cargo.lock` holds otto at the measured version and a
  `cargo update` would move it silently.
  `the_lock_pins_the_indicatif_and_console_this_spike_measured` fails if it does.

### Design decisions

- Two test files rather than one (`tests/progress_spike_pty_test.rs`,
  `tests/progress_spike_indicatif_test.rs`), because they fail for different
  reasons. The pty file goes red when otto's scheduling changes; the indicatif
  file goes red when a dependency moves. Mixing them makes a red suite ambiguous
  about which happened.
- The failing deliverable ships as `#[ignore]`, not `#[should_panic]`:
  `tests/progress_spike_pty_test.rs`,
  `phase_4_no_tty_child_writes_before_every_admitted_task_has_reported`.
  `should_panic` passes for ANY panic, including a fixture that stopped
  measuring anything. Beside it, a `today_`-prefixed twin asserts the race DOES
  reproduce and runs in CI, so the evidence is green and the criterion is
  explicit. Phase 4 removes the attribute.
- `today_`-prefixed tests pin defects rather than approve them:
  `today_a_tty_child_writes_while_otto_is_still_replaying_another_task` and
  `today_an_unterminated_final_chunk_runs_into_the_completion_line`. Each names
  the phase that must invert it. The alternative, leaving the behaviour
  unmeasured until the phase that fixes it, is how a fold lands in one section
  while another keeps describing the old behaviour.
- Benchmarks are counted in bytes and frames, never wall time
  (`per_line_suspension_costs_a_full_region_redraw_for_every_line`). A byte
  count is the same number on a loaded machine and an idle one.
- The handoff fixture widens an existing window rather than manufacturing one.
  `gate` (a 0.05s task) only makes the tty task READY while the buffered group
  still holds every permit; the race itself is otto's, between
  `scheduler/task_execution.rs:500` where the permit drops and `:575` where the
  report is sent.

### Deviations

- The design's Phase 0 bullet lists SIGINT among the behaviours to exercise.
  `tests/sigint_cancel_test.rs:119` already pins terminal Ctrl+C on a plain run,
  and `:441`, `:467`, `:494` pin SIGTERM and SIGHUP. Rather than duplicate them,
  this phase adds the one case none of them covers and the one Phase 4's
  teardown lands in: Ctrl+C while a `tty:` task owns the terminal
  (`a_ctrl_c_while_a_tty_task_owns_the_terminal_ends_the_run_bounded`).
- The design assigns a benchmark to Phase 0 in Technical Considerations >
  Performance ("Phase 0 benchmarks the chatty path before K is chosen") but not
  in the Phase 0 section or its success criteria. It is done anyway, in
  `per_line_suspension_costs_a_full_region_redraw_for_every_line`. See Open
  Questions: the two sections disagree about what Phase 0 is.
- The terminal-hangup test asserts otto does not outlive the pty it was writing
  to, and does not assert an exit code. Killing `script` destroys the master, so
  otto's exit status is not reachable through it; `kill -0` is. The exit-code
  claim belongs to the hangup case that keeps the pty, SIGHUP, already pinned at
  `tests/sigint_cancel_test.rs:467`.

### Tradeoffs

- **A wide fixture over a deterministic one.** The handoff race is a timing
  window, so the fixture widens it (2000 lines per item, three items) instead of
  forcing it with an injection point. Observed 10/10 through a pipe and 5/5
  through a pty. An injected sync point would be production code, which this
  phase does not have, and would prove a race in the injection rather than in
  the scheduler.
- **Byte counts over wall-clock in the benchmark**, which means the numbers
  answer "how much is written" and not "how slow is it". The design's claim is
  about writes (`mp.suspend` "forces a redraw after every callback, bypassing
  indicatif's draw throttle"), so this measures the claim actually made.
- **A test that pins `Cargo.lock`.** It blocks a silent `cargo update` of
  indicatif or console, and it costs a reviewable one-line diff plus a re-run of
  the spike when the bump is deliberate. That asymmetry is the point.

### Open questions

Everything here is for the owner. Phase 0 did not amend the design doc.

1. **`--progress always` is cut by the design's own condition.** Three sections
   still describe it as shipping: API Design (the `always` bullet and the
   `OTTO_PROGRESS` strip rationale), Resolved Decisions ("panel round 1: the
   `always` semantics question resolved"), and Phase 3's
   `--progress <auto|always|never>`. Phase 3 also strips `OTTO_PROGRESS` from
   the child environment specifically because `always` propagates; with `always`
   gone the strip is still right (it stops a parent's `OTTO_PROGRESS=never` from
   muting a nested run's own decision) but the STATED reason no longer holds.
   All four need the fold before Phase 3.
2. **The unterminated-final-chunk defect already exists on main.** The design
   presents it as a hazard the live region introduces ("the region overwrites
   the child's last line"). Measured on `1783f1c` with no region anywhere:
   `[nonl] DONE[nonl] finished successfully`. otto's own completion line is
   concatenated onto the task's last line today. That makes it a bug fix with an
   owner, not a renderer precaution, and it belongs to a phase. Phase 2 is the
   natural home, since the facade is the thing that would track the last byte,
   but which phase fixes it is the owner's call.
3. **Performance and Phase 0 disagree about Phase 0's scope.** Technical
   Considerations > Performance says "Phase 0 benchmarks the chatty path before
   K is chosen"; the Phase 0 section's bullets and success criteria never
   mention a benchmark. One of the two is wrong. (Done anyway, see Deviations.)
4. **`term_like` has no default refresh rate, and the design names
   `term_like`.** Moot for `always`, which is cut. Not moot if a later revisit
   brings `always` back, or if Phase 5 reaches for `term_like` for any other
   reason: the constructor to use is `term_like_with_hz`.
5. **`app.rs:596` is cited as an exit path in Lifecycle and teardown.** `:596`
   is the `eprintln!`; the `std::process::exit` is `:597`. The same paragraph
   cites `:596` correctly as the thing teardown must precede, so this is one
   stale line number in one clause, not a wrong design.
6. **Phase 5's row-style constraint is new** and came out of the version
   verdict, not out of the doc: on 0.18.3 a `MultiProgress` row is styled for
   stdout even on a stderr region. It is written up above; it is not in the
   design doc's Phase 5 bullets.

Everything else checked in this phase held. All five Acceptance Criteria
commands reproduce their recorded output on `1783f1c` exactly, including the
seven-match writer grep across four files and the four `terminal_lock()`
acquisition sites. Every `file:line` citation in the Phase 0, Phase 4 and
Phase 5 bullets, and in Architecture, API Design and Lifecycle and teardown, was
read against the source and is correct, with item 5 above the only exception.

## Phase 1: Remove the heartbeat

`heartbeat.rs` renamed to `clocks.rs` (and its `#[path]`-included test file to
`clocks_tests.rs`). Deleted: `beat_line`, `Heartbeat`, `Shutdown`, `Drop for
Heartbeat`, `spawn`, `wake_granularity`, `MAX_WAKE`, `saturating_ms`, `tick`,
`due_beats`, `emit_due` - the whole ticker, including its terminal write at the
old `heartbeat.rs:429`. `TaskClock` and `TaskClocks` (`note_line`, `note_beat`,
`idle_ms`, `since_beat_ms`, `elapsed`, `candidates`, all of it) are untouched,
per the design doc's instruction to keep `TaskClock` as-is.

### Design decisions

- Followed the ticker's only remaining callers to their source rather than
  stopping at `heartbeat.rs`. `TaskScheduler::start_heartbeat()`
  (`scheduler.rs`) was the ticker's only arming site, and
  `TaskScheduler::progress_interval` (the field) existed only to feed it, so
  both were deleted along with `set_progress_interval()`. That made
  `progress_interval` a dead parameter through `app.rs`'s
  `execute_tasks` -> `execute_with_terminal_output` chain, so it was dropped
  from both signatures and their three call sites. `RuntimeConfig.progress_interval`
  itself (the CLI/config-facing field, still populated from `RunPlan` and
  asserted by `app_tests.rs`) was left alone: it is Phase 3's territory, not
  this phase's, and it is not dead - the test reads it.
- `tests/progress_heartbeat_test.rs` was rewritten rather than deleted or left
  in place. Its dozen wall-clock tests (up to 25s each) asserted the heartbeat
  fires on schedule; with the ticker gone every positive assertion
  (`beats.len() >= 3`, `!beats.is_empty()`) would fail for real, and the
  negative ones (`--progress-interval 0` emits nothing) would pass but for the
  wrong reason - proving nothing is impossible when nothing can ever beat.
  Replaced with three fast (~2s) tests asserting the mirror claim: no `still
  running` line appears at any `--progress-interval` value, including the
  `\r`-redrawn-bar shape the original feature existed for, and that
  `--progress-interval` itself still parses and runs without error (its
  removal is Phase 3's job).
- `scheduler.rs`'s `tick_candidates()` doc comment referenced "Phase 3's ticker
  thread" (the OLD, shipped design's phase); reworded since that thread no
  longer exists. Left the surrounding "heartbeat"/"still running" language
  alone everywhere else it appears only in doc comments or CLI help text
  (`cli/parser.rs`, `cli/parser/help.rs`, `cfg/otto.rs`, `colors.rs`,
  `scheduler_tests_c.rs`) - those describe `progress-interval` CLI/config
  plumbing or historical rationale that Phase 3 owns, and none of them is
  false about code this phase touched.

### Deviations

- None from the design doc's Phase 1 bullets themselves. The cascading
  deletions above (scheduler field/method, `app.rs` parameter threading,
  rewriting the e2e test file) are not named in the doc's Phase 1 section, but
  they are the direct, unavoidable consequence of deleting the ticker the doc
  does name: leaving them in place would have left a dead private field
  (`progress_interval`) and a test suite asserting removed behavior, neither
  of which compiles/passes cleanly.

### Tradeoffs

- Rewrote `tests/progress_heartbeat_test.rs` in place (same filename) rather
  than deleting it and starting a new file. The design doc's own instruction
  ("update or delete any test that asserts a beat line, rather than preserving
  it") reads as license for either; keeping the filename preserves the
  end-to-end coverage this feature's removal deserves without inventing a new
  name only to have Phase 5 possibly need an equivalent file again for the
  live region.

### Open questions

None.

## Phase 2: The output facade

### Design decisions

- `progress::facade` is a module directory (`src/executor/progress/{mod,facade,
  facade_tests}.rs`), not a single file beside `output.rs` — the doc's
  Architecture table already names four siblings (`mode.rs`, `facade.rs`,
  `region.rs`, `ownership.rs`) that later phases add, so the directory exists
  now rather than being a move in Phase 3.
- The facade is reached through a `OnceLock` (`progress/facade.rs::facade`),
  mirroring the `static` lock it replaces, so no scheduler signature changed.
  `Facade::new` stays private so the process instance is the only one
  production code can get; the tests build their own to exercise the ordering
  lock without contending with the harness's captured output.
- `Stream` is a new enum, deliberately NOT `output::OutputType`
  (`progress/facade.rs`). `OutputType` names the stream a *task* produced and
  is `Serialize`d into the run record; `Stream` names one of otto's own two
  handles. Reusing the persisted type would put a display concern inside it.
- Every facade write is best-effort and never panics
  (`progress/facade.rs::write`). `print!`/`eprint!` panic on a failed write,
  which is the hazard `main.rs::report_fatal` already documents for a terminal
  that hung up. The doc's "write failure is best-effort and never fatal,
  matching replay's existing contract" is now true of the live leg too, not
  only of replay.
- `write_replay_blocks` (`scheduler/replay.rs`) holds ONE `facade.block` for
  the whole batch, and the two locked handles are rebound as `out`/`err` locals
  so the body reads exactly as it did under the raw handles. The facade owns
  the lock; it does not own the shape of the writes.
- `report_prune_failure` (`pruning.rs`) became ONE facade write of a
  two-line string instead of two `eprintln!`s. Same bytes, and the two lines
  are one message that a concurrently replayed block must not land between.
- `teardown` is on every exit path even though it only flushes today
  (`progress/facade.rs::teardown`): the call sites are the expensive part to
  find, and Phase 5 should add a renderer, not go hunting for exit paths. It is
  idempotent because the paths overlap — a fatal error during a signalled run
  reaches two of them.
- `tests/output_facade_test.rs` pins the doc's canonical writer grep and the
  `TERMINAL_LOCK` deletion as tests, using the same spellings and the same
  `*_tests*.rs` exclusion. Phase 5 draws a redrawn region on this terminal; one
  ad-hoc writer re-added between now and then is how it gets scribbled over, so
  the criterion needs to keep biting after the phase that introduced it.

### Deviations

- **Facade API is a subset of the doc's.** Shipped: `write`, `block`,
  `teardown`. Not shipped: `task_started` / `task_finished`. They are row
  mutation on a region that does not exist until Phase 5, and no-op methods
  with no callers are dead code the compiler would have to be told to ignore.
  Same seam, added when there is something behind it.
- **Two `auto_prune` sites got teardown, not one.** The doc cites `app.rs:507`
  (the plain path). `execute_with_tui` has its own `auto_prune` call with the
  same `report_prune_failure` hazard, so both got the call.
- **Second-signal teardown sits in `install_stop_handler`, after
  `before_exit()`, not inside the caller's closure.** The doc says "inside the
  `before_exit` closure". One call in the handler covers both callers, and
  placing it AFTER `before_exit()` respects the ordering the existing comment
  at that line already states: the `--tui` caller has to put the primary screen
  back before anything is drawn or erased on it. Same effect, correct seam.
- **Stale line numbers in the doc's Phase 2 bullet, relative to the tree it
  landed in.** `app.rs:507` / `:596` / `:597` were accurate at `1783f1c` but
  Phase 1 (`1526d4c`) removed 26 lines from `app.rs`; the same code is at
  `:481`, `:570`, `:571`. The three acquisition sites (`output.rs:198`,
  `replay.rs:388`, `replay.rs:479`) and `main.rs:235` verified exact.
- **`clocks.rs` had TWO stale references, not one.** The doc names the
  `TERMINAL_LOCK` doc comment at `:43`; there is a second at `:175` pointing at
  `terminal_lock` (`output.rs`) by name. Both rewritten to name the facade.
- The unterminated-final-chunk fix was NOT made here, per the doc's Resolved
  Decision of 2026-09-16. Phase 5 owns it.

### Tradeoffs

- **Byte-identity is asserted over a fixture matrix, not over the whole test
  suite's output.** Method: build the binary at `1526d4c` and at this tree,
  run 12 invocations (chatty/quiet/failing/buffered-foreach/fan-out/fan-out
  with `--no-prefix`/`Graph`/`--help`/`--tasks`/`--list-subtasks`/unknown
  task/missing ottofile) against isolated `OTTO_HOME`s with `NO_COLOR=1`,
  capture stdout and stderr separately, normalize only the run-directory path
  (which embeds a timestamp), and `cmp`. All 36 captures matched. The two
  fan-out fixtures are compared as a sorted multiset of lines, not byte-exact,
  because task interleaving across independent tasks is nondeterministic on
  BOTH binaries: a control run of the Phase 1 binary against ITSELF already
  differed byte-wise there and matched sorted. The control run is part of the
  method, not a footnote — without it, "sorted" would be an unexplained
  weakening of the criterion.
- The facade's unit tests write zero-length strings rather than capturing a
  sink. Testing the ordering invariant needs the lock, not the bytes, and
  injecting a sink would mean the production path and the tested path are
  different code. The cost: `write`'s actual byte delivery is covered only by
  the fixture matrix and the existing end-to-end suite, not by a unit test.
- `a_write_cannot_complete_while_a_block_is_open` asserts from the safe
  direction (a write CANNOT finish while a block is open, so a failure is
  always real; a scheduling hiccup can only make it pass without racing). The
  alternative — sampling a shared flag from outside the write — is what the
  first draft did, and it failed 9 times out of 200 because the sample happens
  after the lock is already released.

### Open questions

- `app.rs`'s second-signal `eprintln!` and `main.rs`'s two error prints are
  still raw macros. They are outside the canonical writer grep's scope
  (`src/executor/`), which looks deliberate, but "the facade is the ONLY
  terminal writer" and "these three are exempt" cannot both be true. If they
  should route too, that is a one-line change per site in a later phase.
- The doc's Lifecycle section lists a FIRST-SIGINT teardown as its own path
  ("`before_exit` does NOT fire here, so this path needs its own teardown
  call"), but Phase 2's bullet enumerates only three sites and does not include
  it. Not added here. On the plain path a first signal still reaches
  `facade.teardown()` through the normal return into `auto_prune`, so nothing
  is currently unguarded; the ordering the doc wants (teardown before
  `flush_cancelled_groups` prints) is a Phase 4/5 concern once there is a
  region to erase.

## Phase 3: Mode plumbing and config migration

`--progress <auto|never>` / `OTTO_PROGRESS` / `otto.progress` now exist,
resolved with the same flag > env > file precedence `-j/--jobs` already uses,
computed once into a `ProgressMode` at startup, and plumbed as far as
`RuntimeConfig` - no renderer reads it yet. `progress-interval` (flag, env,
and ottofile key) is retained, accepted, and now warns on stderr instead of
silently doing nothing. `OTTO_PROGRESS` is stripped from every task's child
environment, on the design's revised rationale: not because `always`
propagates (that spelling is cut), but because the spawn site inherits the
parent's env by design, so a parent's `OTTO_PROGRESS=never` would otherwise
mute a nested otto's own `auto` decision.

### Design decisions

- `ProgressSetting` (`auto`/`never`, what was asked for) and `ProgressMode`
  (`Live`/`Quiet`, what stderr resolves it to) are two enums, not one, in
  `src/executor/progress/mode.rs`. The doc's own Data Model only names
  `ProgressMode`, but the CLI/env/file layer needs a type to parse and
  validate BEFORE stderr is available to consult (`Parser::parse` runs long
  before `RuntimeConfig::from_parser` computes the live-or-quiet decision),
  and collapsing the two would mean either validating against a value that
  does not exist yet or resolving against stderr twice.
- `ProgressMode::resolve` is a thin wrapper over a private, argument-taking
  `resolve_with(setting, stderr_is_terminal, term, ci)`. The real one reads
  `std::io::stderr().is_terminal()`, `$TERM`, and `$CI`; the test-only one
  takes them as arguments, so `mode_tests.rs` can assert every combination
  (tty x dumb x CI) without redirecting the real terminal or mutating process
  env vars other tests in the same binary might read concurrently.
- `OttoSpec::progress` is a raw `Option<String>`, not `Option<ProgressSetting>`
  (`src/cfg/otto.rs`). `executor` already depends on `cfg` (e.g. `Task`), so
  giving `cfg` a `Deserialize` impl that reaches into `executor::progress`
  would cycle the module graph. `Parser::parse` validates the file's string
  through the same `ProgressSetting::parse` the CLI flag uses, so both
  surfaces share one accepted-values list and one error message shape, at the
  cost of the invalid case surfacing at `parse()` time instead of at
  deserialize time (see Tradeoffs).
- The deprecation warning for `progress-interval` fires whenever ANY of its
  three sources set it explicitly (`--progress-interval`,
  `OTTO_PROGRESS_INTERVAL`, or `otto.progress-interval`), not only the
  ottofile key the success criterion names. A user who only ever typed the
  flag would otherwise see no signal that it does nothing.
- `--progress`'s CLI validation is clap's own `PossibleValuesParser`
  (`ProgressSetting::VALID`), the same mechanism `--format` and `--log-level`
  already use in `global_args()`. `always` is rejected before
  `Parser::parse`'s own logic ever runs, with clap's own "possible values:
  auto, never" text, so `ProgressSetting::parse` is in practice reached only
  from `otto.progress` (confirmed by the `expect` at the CLI call site,
  which has never fired in any test run).
- The nested-otto success criterion is tested against an ACTUAL nested otto
  process (`tests/progress_mode_test.rs`,
  `a_nested_otto_sees_no_otto_progress_in_its_environment`), an outer task
  invoking the same binary under test as its own child, rather than only
  asserting the strip on a single task's environment. The single-task case
  would already prove the mechanism, but the criterion's own wording is about
  a nested otto specifically, and cargo's `CARGO_TERM_PROGRESS_WHEN` hazard
  this design cites is exactly the nested-process case.

### Deviations

- **`OttoSpec::progress` is `Option<String>`, not a typed enum**, unlike
  every other otto-owned enum-shaped key in this codebase (e.g. `Nargs`,
  `When`). Recorded here because it reads as an inconsistency without the
  module-cycle reason above written down next to it.
- **The design doc names no test file for Phase 3**; `tests/progress_mode_test.rs`
  is new, alongside additions to `src/cli/parser_tests_a.rs` (unit-level
  precedence: flag > env > file, both directions, plus the invalid-value
  cases) and `tests/roundtrip.rs` (serialize/deserialize fidelity for the new
  key, mirroring the existing `progress-interval` round-trip tests).
- **`docs/commands/ottofile-reference.md` and its cross-checking test
  (`ottofile_reference_key_inventory_is_exhaustive`,
  `src/cfg/task_tests.rs`) were updated**, even though the design doc assigns
  README/`docs/commands/` updates to Phase 5. Not optional here: that test
  destructures `OttoSpec` and asserts its on-disk key count and names against
  the reference doc, so adding `otto.progress` without documenting it is a
  compile-and-test-time failure, not a style choice deferred to a later
  phase. `README.md` itself is untouched (see Open questions): nothing forces
  it, and its `--progress-interval` example belongs to Phase 5's mandate.
- **The exact CLI exit code for a rejected `--progress` value is 1, not
  clap's own 2.** otto's `main.rs` converts every `Parser::parse` `Err` -
  including a clap `PossibleValuesParser` rejection wrapped in an
  `eyre::Report` - into exit code 1 (`main.rs`'s catch-all), rather than
  letting clap's own `try_get_matches_from` print and exit(2) itself. This is
  pre-existing behavior (an out-of-range `-j` value exits the same way) and
  not something Phase 3 changed; noted because a first draft of
  `tests/progress_mode_test.rs` asserted exit code 2 and was wrong.
- **The `progress-interval` help text update cited at `src/cli/parser/help.rs:72`
  resolved to line 69-75** in the Phase-2 tree (the citation drifted, as the
  doc's Implementation Plan preface warns it would); re-located by symbol
  (`Arg::new("progress-interval")`), not by line number.

### Tradeoffs

- **`otto.progress`'s invalid-value error surfaces at `Parser::parse()`,
  not at YAML-deserialize time**, unlike `otto.jobs`'s `0` rejection
  (`deserialize_jobs`, `src/cfg/otto.rs`). The alternative (a custom
  `Deserialize` on a `cfg`-local newtype mirroring `ProgressSetting`) would
  duplicate the accepted-values list and its error text in two places that
  could drift, which is the exact npm `5b858c6` lesson this design cites
  elsewhere; deferring validation one layer up keeps one list and one
  message, at the cost of a bad ottofile value surfacing very slightly later
  in the same `Parser::parse` call than `otto.jobs: 0` does.
- **The deprecation warning is a plain `eprintln!` in `src/cli/parser.rs`,
  not routed through the Phase 2 facade.** The canonical writer grep in
  Acceptance Criteria is scoped to `src/executor/`, and `parser.rs` already
  prints its own config-load errors the same way (`eprintln!("Error: {e:#}")`
  a few lines above), so this matches the file's existing convention rather
  than reaching into `executor::progress::facade` from `cli` for a
  startup-time, one-shot warning that happens before any live rendering
  could be in flight regardless.
- **One `#[test]` fixture per success criterion, plus process-level and
  unit-level coverage of the same precedence rules**, rather than only the
  three criteria the doc names. The unit tests in `parser_tests_a.rs` run in
  microseconds and pin the flag/env/file precedence directly; the
  process-level tests in `progress_mode_test.rs` are slower but are what
  actually proves the CLI's clean-error behavior and the nested-otto
  environment, which no unit test can observe.

### Open questions

- `README.md`'s Usage section still shows `otto --progress-interval 5 build`
  as if it does something, and its flag table has no `--progress` row. The
  design doc assigns README updates to Phase 5 (alongside flipping the
  superseded heartbeat doc's Status); left untouched here rather than
  partially updating it out of order. Confirm Phase 5 still owns this.
- The deprecation warning's exact wording and channel (a bare `eprintln!`,
  once per invocation, naming all three deprecated spellings) was not
  specified by the design doc beyond "warned on". If a structured warning
  (e.g. routed through `log::warn!` as well, for otto's own log file) is
  wanted, that is a one-line addition here or in a later phase.
- `rg -c 'progress[-_]interval|PROGRESS_INTERVAL' src/ tests/` now reads 84
  lines, down from 98 on `main` at `1783f1c`. It does not reach zero, by
  design: the key is deprecated for one release, not removed, and this
  phase's own new test file (`tests/progress_mode_test.rs`) legitimately
  names both spellings in its warning-text assertions. The count should drop
  further only when a later release removes the key outright.

## Phase 4: Terminal-ownership gate

Two independent pieces, both of them scheduler-and-facade work with no
renderer anywhere:

- the admission gate in `src/executor/scheduler.rs` (`may_admit`, `InFlight`,
  `is_bookkeeping`, and a drain-mode flag local to `execute_all`).
- the handoff in `src/executor/progress/ownership.rs`, reached through
  `Facade::surrender` / `Facade::reclaim`, taken around the `tty:` spawn in
  `src/executor/scheduler/task_execution.rs`.

### Design decisions

- **`InFlight` gained a `total` field rather than a new counter beside it** —
  `scheduler.rs:ActiveTasks::in_flight` — the count is `self.running.len()`,
  the same map the other two arms project, so the tty rule reads the same
  snapshot as the rules it sits beside and cannot disagree with them. The doc
  named `in_flight_len` as the right shape; this is that number, delivered
  through the struct `may_admit` already takes so the function stays pure and
  table-testable.
- **Drain mode is a `bool` local to `execute_all`, passed into `may_admit` as
  its third argument** — `scheduler.rs:execute_all` — so the whole admission
  decision is still one pure function with one table test
  (`drain_mode_stops_ordinary_tasks_overtaking_a_deferred_tty_task`), rather
  than a predicate plus an `if` somewhere in a 400-line loop. It is set in
  BOTH deferral branches (the cap branch and the ownership branch) and cleared
  when a tty task is admitted or when nothing is left in flight.
- **Drain mode ends when nothing is in flight, whether or not the tty task
  went in** — `scheduler.rs:execute_all`, top of the launch pass. A tty task
  restored to the head of the queue can still be skipped by a gate on its way
  back through, and a flag left waiting for a task that is never coming would
  stop the loop admitting anything for the rest of the run.
- **Drain mode covers `Admission::Capped` only, which is exactly the doc's
  wording ("stops admitting ordinary tasks")** — `scheduler.rs:may_admit`.
  Applying it to exempt items as well compiled, passed the two Phase 4
  criteria, and broke `foreach_jobs_concurrency_test.rs`'s
  `a_tty_task_and_an_exempt_group_never_overlap_at_one_job`: with a tty task
  waiting, ONE item of a `jobs: all` group started and the other two were
  deferred, which is `foreach.jobs`'s whole promise broken. An exempt item
  overtaking a ready tty task is also not the hole drain mode exists to close:
  it is what otto did before this phase, and a group is finite, so the tty
  task's wait is bounded by it either way.
- **`is_bookkeeping` — a virtual parent with an empty action — is the one
  thing a tty task yields to, and the one thing drain mode does not hold** —
  `scheduler.rs:is_bookkeeping`, used twice in `execute_all`. Found by running
  the Phase 0 criterion test against the bare predicate: it still failed, with
  `OWNER-START` at line 6004 and `[bulk] finished successfully` at 6005. The
  mechanism is that a foreach group's virtual parent becomes ready only once
  every subtask has reported, so at that instant `in_flight.total == 0` and the
  tty task - sitting at the head of the queue after its earlier deferral - is
  admitted ahead of the group's own completion line. The parent runs no script
  (`as_virtual_parent`, `src/cfg/task.rs`, sets `action: String::new()`), so
  yielding to it costs one replenishment and cannot reintroduce the
  unbounded-delay regression drain mode exists to prevent. Drain mode must
  exempt it or the two rules deadlock against each other: the tty task waits
  for the parent, and drain mode would hold the parent.
- **The handoff is an RAII guard (`TerminalHandoff`), not an acquire call
  paired with a release call** — `ownership.rs`, taken at
  `task_execution.rs`'s `tty` arm. Three of the four ways out of that spawn
  (spawn failure, cancellation dropping the body mid-`wait`, and a `?` on the
  log-marker write) never reach a line an author could have put a release on.
  `Drop` reaches all four.
- **Ownership is state on the `Facade`, not a second static** —
  `facade.rs:Facade::terminal`. Every site takes `order` first and `terminal`
  second, so a handoff cannot land in the middle of a block: `surrender`
  itself blocks until whatever replay was mid-write has finished, which IS the
  handoff the design's diagram describes.
- **Held output is drained by `teardown` as well as by the guard** —
  `facade.rs:Facade::teardown`. A run that exits while a `tty:` task still
  owns the terminal has held bytes nobody else will flush, because the body
  that would hand ownership back is being dropped or already was. Teardown is
  the last writer on every exit path (Phase 2 put it there), so it is also the
  last chance. Pinned by
  `teardown_hands_the_terminal_back_rather_than_leaving_output_held` and,
  end to end, by
  `a_cancelled_run_still_prints_its_notice_with_the_terminal_surrendered`.
- **The spill is framed (`tag | u64 len | bytes`) and re-streamed in pieces**
  — `ownership.rs:Surrendered::spill_write`, `drain_spill`. otto's two streams
  interleave inside one replayed block, so a plain concatenation could not be
  put back on the right handle; `io::copy` over a `take(len)` keeps re-arm's
  memory cost at the copy buffer rather than the record size.

### Deviations

- **The doc's "a surrendered replay spills to its own per-task log file" is
  implemented as a spill file in the TTY task's run directory
  (`<run>/tasks/<tty task>/otto-surrendered.log`), not as a deferral that
  re-reads the replayed subtask's `stdout.log`.** Same effect, correct seam:
  the facade is byte-oriented and has no task identity, so the deferral
  reading would have to plumb re-stream closures (or block descriptors) back
  out through the scheduler and find a thread to run them on at re-arm.
  Everything the doc asked for holds: memory is bounded by
  `SURRENDER_MEMORY_BYTES` (64 KiB, replay's own chunk bound), nothing touches
  the streaming path while the terminal is surrendered, and the bytes are
  re-streamed from a file that is deleted once they are.
- **The tty gate additionally yields to a ready bookkeeping parent**, which
  the doc's Phase 4 bullets do not mention. It is not extra scope: it is what
  the doc's own Phase 0 criterion test demands, and without it that test
  fails. Recorded here because it is a rule the doc does not state.
- **`may_admit(Tty, ...)` now also refuses a second tty task** (`total == 0`
  subsumes the old `exempt == 0`, and `tty` is part of `total`). The old rule
  admitted a tty task beside another tty task and let the shared semaphore
  serialize them. Nothing depended on that: it is one terminal, and the test
  that pinned the old cell,
  `the_admission_rules_are_symmetric_and_bind_only_tty_against_exempt`, was
  renamed `a_tty_task_waits_for_every_admitted_task_to_report` and has the
  inverted cells called out inline rather than being left green by accident.
- **`today_a_tty_child_writes_while_otto_is_still_replaying_another_task` was
  inverted and renamed** to
  `a_tty_child_no_longer_writes_while_otto_is_still_replaying_another_task`,
  per the rule at the top of `tests/progress_spike_pty_test.rs` ("A later
  phase that fixes the behaviour must INVERT the named test, not delete it").
  Its criterion twin was un-ignored in the same file.
- **`Facade::new` went from private to `pub(super)`** so `ownership_tests.rs`
  can build a private facade instead of mutating the process-wide one, which
  is the same reason `facade_tests.rs` already built its own.

### Tradeoffs

- **Spill file vs. deferring the replay** — a spill costs disk equal to the
  held output and one extra copy of those bytes, where deferring costs
  nothing but needs the replay layer to become ownership-aware. Chosen for the
  spill because the only reachable path that can produce a mid-surrender
  replay today is the cancellation flush (the admission gate closes every
  other one), and paying bytes on a cancellation path is better than adding an
  ownership concept to `replay.rs` that the gate makes almost unreachable.
- **Held output is bounded in memory but NOT bounded overall** — past a failed
  spill, `Surrendered::overflow` grows without limit. Deliberate: the
  alternative is dropping a failure status on a filesystem that is already
  broken, and the condition is reported on stderr at re-arm rather than
  swallowed (`a_spill_that_cannot_be_opened_keeps_the_output_and_reports_itself`).
- **Surrender is unconditional, not gated on `ProgressMode`** —
  `task_execution.rs`. In `Quiet` (piped) runs otto's own status lines are now
  held for the duration of a `tty:` child rather than interleaving with its
  inherited output. That is the better ordering, it costs nothing measurable,
  and gating it on the mode would give the two paths different terminal
  semantics for no stated reason.
- **The drain-mode fixture is built out of dependency edges, not declaration
  order** — `tests/tty_admission_gate_test.rs`. The run set's order is a
  `HashMap` iteration order: measured 2026-09-17, four independent tasks
  started in two different orders on two consecutive runs of the same
  ottofile. The fixture uses two gate tasks so that `[tty, long-1, long-2]` is
  the queue the pass actually sees; the first draft assumed declaration order
  and was testing the allocator.
- **No integration test for a failed `tty:` spawn.** The interpreter is only
  ever `bash` or `python3` (`ProcessedAction`, `src/executor/action.rs`), so a
  spawn failure cannot be provoked portably from an ottofile. The path is
  covered structurally instead: the guard is taken before `spawn()` and
  released by `Drop`, and `the_facade_holds_writes_while_the_terminal_is_surrendered`
  pins that dropping it hands the terminal back with no other call involved.

### Open questions

- **Buffering while surrendered is now almost unobservable end to end, by
  construction.** The gate admits a `tty:` task only when nothing else is in
  flight, so the only otto writes that can land during a surrender are a skip
  line for a task the loop retires while the child runs, and the
  run-cancelled notice. The second is tested
  (`a_cancelled_run_still_prints_its_notice_with_the_terminal_surrendered`);
  the rest of the hold-and-replay contract is covered by unit tests on
  `Surrendered`. Confirm that is the intended division rather than a missing
  integration test.
- **Citations re-located; one in the doc no longer resolves as written, and
  one in the Phase 0 test file does not either.** Everything the phase relied
  on was found by symbol, against the tree as committed by this phase:
  `scheduler.rs:342` (`Admission::Tty => in_flight.exempt == 0`) is now `:366`
  (and reads `total == 0`); `:348` (`Capped => true`) is `:392`; `:408`
  (`in_flight_len`) is `:465`; `:484` (`reported`) is `:544`; `:317-321`
  (`InFlight`) is `:326-337`; `:1638-1639`
  (`deferred_by_admission.push(task); continue;`) is `:1653-1654`;
  `:1662-1663` (restore to the head) is `:1718-1721`; `:363-367`
  (`admission_for`) is `:418-426`; `task_execution.rs:298-299` (the inherited
  stdout/stderr) is `:324-325`.
  **`scheduler.rs:1524-1530`, the shared run-start `Instant` the doc cites for
  Phase 5, is now `:1531-1541`** - it resolves by symbol, but Phase 5 must
  re-locate it rather than trust the number.
  **`replay.rs:389-391`, cited by the Phase 4 bullet as "that path takes real
  stdout/stderr handles", no longer resolves as written**: Phase 2 replaced
  the raw handles with `facade().block(...)`, now at `replay.rs:393`, and the
  handles live inside `Facade::block`. The claim is still true and is exactly
  why the hold does not spill to the streaming path; only the address is
  stale. The Phase 0 header comment in `tests/progress_spike_pty_test.rs` also
  cites `scheduler.rs:363-367` for "a buffered foreach without `jobs`
  classifies `Capped`", which is `admission_for`, now `:418-426`; left as
  written, since that file is Phase 0's record and the line numbers in it were
  true when it was measured.
