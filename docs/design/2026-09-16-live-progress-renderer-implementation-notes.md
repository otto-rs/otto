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
