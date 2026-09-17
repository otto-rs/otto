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
