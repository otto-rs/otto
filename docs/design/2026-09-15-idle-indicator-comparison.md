# Comparison: idle heartbeat vs PR #9's idle-activity spinner

**Date:** 2026-09-15
**Compares:** `docs/design/2026-09-15-idle-task-heartbeat.md` against
`otto-rs/otto` PR #9 (`idle-activity-spinner`, Ian McEachern, draft, +782/-4).

The two designs were derived separately. The heartbeat doc was written from the
Slack ask, otto's source, and a survey of how cargo, bazel, docker, npm, gh,
terraform, rich, tqdm and indicatif handle this. PR #9 was read afterward.

## The one decision everything else follows from

| | PR #9 | Heartbeat |
|---|---|---|
| Mechanism | `\r` + `\x1b[K` frame redrawn in place | `\n`-terminated line |
| Visible when | stderr is a tty, and 5 other conditions pass | always |
| Log safety comes from | a predicate that must stay correct forever | there being no `\r` and nothing redrawn in place |
| Shows anything in CI, or when stderr is captured (`2>&1 \| tee`, `2>log`, nested otto) | no | yes |
| Shows anything under `\| tee log` (stdout piped, stderr still a tty) | yes | yes |
| Adds lines to an interactive terminal | no | yes, one per stalled task per interval |

The last row is the honest cost of the heartbeat and the honest win for PR #9.
Everything below is secondary to it.

An earlier draft of this table put "no" in PR #9's column for `tee`. That was
wrong: under `otto task | tee log` stderr is still a terminal and `should_enable`
(pr9.diff:233,531) draws. Only `2>&1 | tee` suppresses it. Corrected after
review.

**The decider, which neither design doc originally stated.** The drain reads
`read_until(b'\n')` (`src/executor/output.rs:250`). A tool rendering a
`\r`-redrawn progress bar emits no newline, so it never yields a line, never
resets an idle clock, and is heartbeated by A. `philo slim-dump`, the motivating
case, is that shape. B's `is_terminal` gate shows nothing for that same case in
CI or a captured log.

## Where PR #9 is right, and the heartbeat has nothing better

- **`TerminalGuard`.** Moving the erase discipline onto `terminal_lock()`
  instead of spreading an "erase before you print" convention across call sites
  is the correct construction for an animated indicator. It is the best idea in
  the PR. It is also not harvested here, because the heartbeat has nothing to
  erase; see Recommendation.
- **`should_enable` as a pure function taking its environment as a closure.**
  Every row of the suppression table becomes a unit test instead of a
  process-level `set_var` experiment. It follows `choose_format`'s existing
  shape.
- **`tests/progress_spinner_test.rs`.** Asserting on the bytes of a captured run
  is exactly the test lefthook lacked before issue #1539. The heartbeat doc
  copies the test and narrows its escape-byte assertion to stderr, because
  otto's stdout legitimately carries colour whenever stdout is a terminal.
- **Width truncation with a spare column.** The reasoning (a wrapped frame
  occupies two rows while the erase clears one) is correct and is the failure
  most hand-rolled spinners ship with.
- **Stderr-only, never `/dev/tty`.** Correct, and correctly argued from otto's
  own `setsid` invariant.

## Defects in PR #9, verified against the diff and otto's source

1. **It hooks the wrong in-flight ledger.** `progress::started()` is called from
   `src/executor/scheduler/support.rs:64`, inside the launch loop. That is
   admission time. `Admission::Exempt` tasks (a foreach group that declared
   `jobs:`) are launched unbounded and throttled by a per-group semaphore inside
   the body (`task_execution.rs:51-54`), so on a `foreach ... jobs: 4` over 40
   items the frame reads `40 running: …` while 4 processes exist. The truthful
   ledger is `register_child` / `deregister_child` (`scheduler.rs:258`,`:277`).

2. **The clock is the run's, not the task's.** `configure()` sets
   `State.started` once, and `frame_line` renders `st.started.elapsed()`. So
   `⠋ philo · 4:12` means the run has been going 4:12, not philo. The PR's own
   example (`⠋ quiet · 0:06`) reads as task elapsed and coincides only because
   the run has one task that starts at run start. The request asked for the
   elapsed time of the thing that looks stuck.

3. **A single global idle clock answers a different question.**
   `after_write()` resets one `State.last_output` for the whole run
   (pr9.diff:483,632), so with `-j` at the CPU count a chatty task suppresses
   the spinner while a sibling is wedged. Both review seats confirmed the fact
   and split on severity. It is a scope difference rather than a defect: B
   answers "is the run silent?", A answers "is this task silent?". Calling it
   "the reported failure reintroduced" overstated it, and that wording is
   withdrawn.

4. **`clear()` documents a panic hook that the PR does not install.** Its doc
   comment says it is called on "normal return, signal handler, and the panic
   hook". The diff adds two `install_stop_handler` callbacks and one call after
   `execute_all`, and touches no panic hook; `install_panic_hook` remains
   TUI-only in `src/tui/mod.rs`. A panic mid-run under a tty leaves a frame on
   screen, which the module's own comment calls "the only damage this feature
   can actually do".

5. **Shutdown races with drawing.** `draw_if_idle()` checks `enabled()` before
   taking `STATE`; `clear()` flips `ENABLED` and erases while holding only
   `STATE`, never the terminal lock (pr9.diff:655-663). A ticker can pass the
   enabled check, be preempted, let `clear()` finish, then take `STATE` and
   redraw. The loop exits with a frame on screen, which is the one outcome the
   module's own comment calls the only damage the feature can do.

6. **`NO_COLOR` disables the animation.** That conflates colour with motion.
   `console`'s own colour predicate reads `CLICOLOR` / `CLICOLOR_FORCE`;
   indicatif gates on `is_term()` and `is_dumb()` and ignores `NO_COLOR`
   entirely. Someone who wants plain text still wants to know the thing is
   alive.

7. **Thresholds are hardcoded.** `IDLE_BEFORE_SPIN` and `TICK` are consts. The
   thread left the value open ("5 (3?) seconds") and `taste.md` is explicit that
   tunables ride the standard delivery path rather than being baked in. otto has
   the `jobs` pattern (`Option` on `OttoSpec` + `value_source`) sitting right
   there.

8. **`TERM` unset disables the spinner outright.** Stricter than cargo,
   indicatif or console, which special-case only `dumb`.

9. **CI has never run on it.** `gh pr checks 9` reports "no checks reported on
   the 'idle-activity-spinner' branch"; `statusCheckRollup` is empty. It is a
   fork PR awaiting maintainer approval. Every verification claim in the PR body
   is a local run.

Items 1, 2, 4 and 5 are behavioural and would ship wrong. Item 3 is a scope
difference, not a defect. 6 through 8 are taste and convention. 9 is a process
step, not a defect in the code.

**Withdrawn after review.** An earlier draft listed "`--no-progress` is a boolean
flag" as a defect, citing `taste.md`. That misreads the rule, which prohibits
boolean *format* flags, not all boolean opt-outs (`rules/taste.md:95`). The
reviewing Staff Engineer declined to flag it and was right.

## Where the heartbeat is exposed and PR #9 is not

- **It adds lines to an interactive session.** A 5-minute silent download at the
  default 10s interval prints 30 lines. PR #9 prints zero and animates in place.
  If that turns out to matter in practice, the animation is the upgrade, and the
  heartbeat's per-task clock is what it would need anyway.
- **It cannot be hidden retroactively.** An escape-based spinner leaves nothing
  behind; a heartbeat line is in the scrollback.
- **Its own "no escape bytes" claim was false** in the first draft of the design
  doc, and is corrected there. `colored` derives colour from
  `stdout().is_terminal()` (`colored-3.1.1/src/control.rs:107`) and otto applies
  it to stderr, so `otto boom 2>log` with stdout on a terminal already writes
  6 escape bytes into `log` on `main` today. The heartbeat gates its own label to
  avoid adding to that; it does not fix it.
- **PR #9 exists.** The heartbeat is a document.

## Recommendation

Build the heartbeat, and harvest `tests/progress_spinner_test.rs` almost
verbatim.

Not harvested: `TerminalGuard`. It is the right construction for an animated
indicator, and the heartbeat has nothing to erase, so taking it now would be
scope carried for a feature that is a non-goal. It stays recorded in the
revisit condition instead.

The deciding argument is not that the spinner is badly built. It is the
`read_until(b'\n')` point above: the exact case that prompted the request is one
where an `is_terminal`-gated animation correctly decides to show nothing, in CI
and in any run whose stderr is captured. A secondary argument is that the
animation's log safety rests on a predicate every future contributor has to keep
correct, across six environment conditions and every new terminal writer.

One claim this document previously made without a source, now withdrawn: that
the animated path's obligations are unusual. They are standard for the category;
cargo, gh and docker all carry them successfully.

If the interactive-noise cost is judged too high, that reverses the
recommendation, and PR #9 with defects 1 through 4 fixed is the right base.
That is a call for the owner.
