# Design Document: Live Progress Renderer

**Author:** Scott Idler
**Date:** 2026-09-16
**Status:** Implemented. All five phases landed (`527d781`, `1526d4c`,
`afdbafe`, `ef882bb`, `0d888c7`, and Phase 5), each with its own commit and its
own section in
`docs/design/2026-09-16-live-progress-renderer-implementation-notes.md`.
`--progress always` was measured and CUT by Phase 0; the flag ships
`auto | never`. Five authoring passes, three panel rounds, all findings
dispositioned, Open Questions empty.
**Review Passes Completed:** 5/5

## Summary

Replace the ungated `still running` heartbeat line (v2.5.3) with two renderers
chosen by whether stderr is a terminal: a live in-place region on a tty, and
nothing periodic when captured.

**This does not fix the motivating case.** Under OQ1 option A, `philo
slim-dump` run as `2>&1 | tee` stays completely silent for its whole
execution. The renderer answers the interactive complaint, not the captured
one. Stated here and not only under OQ1, because a reader who skims the
Summary should not have to discover it in an appendix.

Supersedes `docs/design/2026-09-15-idle-task-heartbeat.md`. Phase 5 flips that
doc's Status to `Superseded by docs/design/2026-09-16-live-progress-renderer.md`;
a doc left saying `Implemented` about code that no longer exists is a lie the
next reader will act on.

## Problem Statement

### Background

- Ian asked for a spinner so a silent long task does not look frozen. His
  constraint: "If there's no way to show the spinner without defacing logs then
  it's not worth the visual indication."
- v2.5.3 shipped a heartbeat LINE instead, deliberately ungated, on the theory
  that a captured log "is where 'is it hung?' gets asked most."
- Ian ran it against `philo slim-dump` and it failed in production.

### Problem

Two defects, both reported with verbatim output, both confirmed in the code.

- **Doubled beats under nested otto.** An otto task whose body runs otto
  produces two beats per interval: the inner heartbeats its own stderr, the
  outer reads that as task output and re-prefixes it, and the outer beats too.
  Nothing suppresses a child otto's renderer. `nested` appears ZERO times in
  the shipped design doc: a blind spot, not an accepted tradeoff.

  ```
  [data:philo] still running (3m25s)
  [data:philo] [data] still running (3m24s)
  [data:philo] still running (3m35s)
  [data:philo] [data] still running (3m34s)
  ```

- **The count never resets between idle periods.** `beat_line` prints
  `clock.elapsed()` = `started.elapsed()` (`src/executor/heartbeat.rs:107`,
  `:399`): total task runtime. The idle clock (`idle_ms`, `:97`) decides
  WHETHER to beat but never reaches the text. Ian read the number as silence
  duration.

Scott's verdict on the shipped feature: "yeah I hate this". Ian: "lol nuke it
then ... I was hoping for an easy spinner ... if not easy not worth it".

### This reversal was pre-authorized

The shipped design rejected the tty-gated spinner (its Alternative 1) with an
explicit revisit condition at
`docs/design/2026-09-15-idle-task-heartbeat.md:671`:

> "a user running otto interactively reports the heartbeat lines are too noisy
> in practice. The animation is then a strictly additive upgrade on the tty
> path, and the ledger and idle clock built here are exactly what it needs."

That condition fired on 2026-09-16. It is also correct about the salvage: the
ledger and idle clock survive this change untouched, so Phase 2 of the shipped
work is reused, not thrown away. What the shipped doc got wrong was scope, not
direction: it treated tty gating as purely a visibility tradeoff and never
noticed it was also the nesting fix.

### Root cause, both defects

Not tty-gating. That single decision produced both:

- gating on stderr would make the inner otto silent, because its stderr is a
  pipe. No guard var needed.
- a one-shot line has room for one number, forcing elapsed | idle. A redrawn
  row has room for both, so the choice evaporates.

### Goals

Every goal traces to who asked for it. Nothing here is unrequested scope.

| goal | asked by | where |
|---|---|---|
| interactive liveness for a silent task | Ian | "can you add a spinner indicator when an action is in flight" |
| zero new bytes in a captured stream | Ian | "If there's no way to show the spinner without defacing logs then it's not worth the visual indication" |
| nested otto emits one renderer | Ian | "when delegating otto to another otto task we get doubled up ones" |
| duration names its subject | Ian | "the count does not reset between 'idle' periods" |
| one renderer owns the terminal, enforced | Staff Engineer | 2026-09-16 design review: the `:500`/`:575` permit gap |
| facade, not ad-hoc writers | Staff Engineer | same review: "retain an otto-owned output facade" |
| duration on the completion line | follows from OQ1=A | with no periodic output, the completion line is the ONLY place a captured run learns how long something took |

### Non-Goals

- **Periodic liveness in a captured stream.** Excluded, see Open Question 1.
- **A per-task progress override.** Excluded: tty gating removes the case that
  motivated it. Revisit condition: a user needs liveness for SOME captured
  tasks and not others.
- **`--tui` dashboard changes.** Separate surface, untouched. The facade
  must respect `suppress_terminal` exactly as `TeeWriter` does today
  (`output.rs:187`), so a TUI run stays silent on the plain streams.
- **Progress for `tty:` tasks.** They own the terminal; otto stands down.
- **OSC 9;4 / terminal-title progress.** Parked. Revisit condition: someone
  asks for taskbar progress. Aggregate-only, cannot carry per-task rows.

## Proposed Solution

### Overview

- One decision, computed ONCE at startup: does stderr take a live renderer?
- `auto` (default) | `never`. Cargo's `term.progress.when` also offers
  `always`; Phase 0 measured it and CUT it, see API Design below.
- tty -> a bounded live region on stderr, one row per running task, redrawn in
  place. Zero lines added to scrollback.
- not-tty -> no periodic output at all. Event lines only, with elapsed on the
  completion line.
- All otto terminal writes go through an otto-owned facade. indicatif sits
  BEHIND the facade, never called directly from the writing sites.
- While a `tty:` task owns the terminal, otto's own writes are buffered, not
  dropped, under a bounded buffer that spills to per-task log files rather
  than to the terminal.

### What the user actually sees

**Live path** (`otto data`, stderr is a tty). Rows redraw in place at the
bottom; nothing is appended to scrollback except real task output.

```
[data:philo] pg_restore: processing data for table "public.clearance_predictions"
[data:api]   Compiling philo v0.42.1
--------------------------------------------------------------
philo    running 4m05s   no output for 41s
api      running 1m12s
catalog  running 0m08s
... and 2 more running
```

**Quiet path** (`otto data 2>&1 | tee build.log`, or CI). Byte-for-byte what
otto prints today MINUS the beat lines, PLUS elapsed on the completion line.

```
[data:philo] pg_restore: processing data for table "public.clearance_predictions"
[data:api]   Compiling philo v0.42.1
[api]   finished successfully  1m54s
[philo] finished successfully  41m22s
```

The completion line keeps its current wording (`status_label` +
`task_outcome_word`, `scheduler.rs:1782-1785`) and gains a duration. Only the
duration is new.

**Which path each invocation takes.** The renderer lives on stderr, so a
stdout redirect does NOT silence it. Verified by pty probe.

| invocation | stderr | path |
|---|---|---|
| `otto data` | tty | Live |
| `otto data > out.log` | tty | Live, and `out.log` gets zero escape bytes |
| `otto data \| tee log` | tty | Live |
| `otto data 2> err.log` | file | Quiet |
| `otto data 2>&1 \| tee log` | pipe | Quiet |
| inner otto of a nested run | pipe | Quiet |
| CI | tty or pipe | Quiet (`auto` also checks `CI`) |

Under OQ1 option A, a silent `philo` contributes NOTHING between its last line
and its completion line. That is the whole product contract and it is why OQ1
is Scott's call, not the author's.

### Architecture

| Concern | Home | Note |
|---|---|---|
| renderer decision | `src/executor/progress/mode.rs` | computed once, stored, never re-derived |
| the facade | `src/executor/progress/facade.rs` | the ONLY terminal writer |
| live region | `src/executor/progress/region.rs` | indicatif `MultiProgress` behind the facade |
| terminal ownership | `src/executor/progress/ownership.rs` | handoff for `tty:` tasks |
| idle clock | `src/executor/heartbeat.rs` | KEPT, renamed `clocks.rs`, beat removed |

**The facade is the load-bearing piece.** `TERMINAL_LOCK`
(`src/executor/output.rs:40`) is deleted, because indicatif's ticker thread
writes to the terminal holding only indicatif's locks and never otto's, so a
retained `TERMINAL_LOCK` would serialize otto's writers against each other but
not against a tick. The facade owns the replacement invariant:

- every write is `facade.write(...)` | `facade.block(|w| ...)` for a contiguous
  run. No caller touches `print!`, `eprint!`, or a `ProgressBar`.
- the facade recovers a poisoned lock the way `TERMINAL_LOCK` did today
  (`output.rs:48-49`). indicatif unwraps; the facade catches, so a render
  failure degrades to plain writes instead of poisoning every later writer.
- write failure is best-effort and never fatal, matching replay's existing
  contract. `EIO` after terminal hangup is already handled at `main.rs:240-249`.

**Terminal ownership handoff.** `tty: true` tasks inherit stdout/stderr
(`scheduler/task_execution.rs:298-299`) and write outside any otto lock. The current
semaphore does NOT cover this: `_permit` is bound at `scheduler/task_execution.rs:86`
inside the `outcome` future which ends at `:500`, but the report is not sent
until `:575`. A queued `tty:` task can take every permit in that window while
the parent still has a status line or a buffered replay to print. The handoff
closes it:

```
parent: acquire ownership  -> erase region, stop ticker, mark surrendered
      : spawn tty child
child : owns the terminal
      : reap
parent: reset defensively (\x1b[0m), re-read winsize, re-arm region
```

While surrendered, the facade BUFFERS otto-authored writes rather than dropping
them, and flushes on re-arm. Dropping would lose a failure status.

### Data Model

```rust
/// Computed once at startup from stderr. Never re-derived: npm shipped the bug
/// where the predicate was computed twice and diverged (npm/cli 5b858c6).
pub enum ProgressMode { Live, Quiet }

/// Holds the EXISTING clock. It does not keep its own `started`: a third
/// independent start instant is how the live row and the completion line
/// would drift apart again (see Phase 5).
pub struct TaskRow {
    name: String,
    clock: Arc<TaskClock>,  // .elapsed() -> "running 3m34s"
}                           // .idle_ms() -> "no output for 35s"
```

`TaskClock` (`heartbeat.rs:37-112` at `1783f1c`; the module is `clocks.rs`
from Phase 1 onward) is kept as-is. `note_line`, `idle_ms`,
`elapsed` all survive; only `beat_line` and the ticker's emit path go.

### API Design

```
--progress <auto|never>            default: auto
OTTO_PROGRESS=<auto|never>
otto.progress: auto|never          (ottofile)
```

- `auto`: Live iff `stderr.is_terminal()` AND `TERM != dumb` AND no `CI` env.
- `always`: **CUT by Phase 0 (`527d781`), on this design's own condition.** The
  draw-path half held: `TargetKind::TermLike` (`draw_target.rs:198`) carries no
  terminal predicate, unlike `TargetKind::Term` at `:176`, and `TermLike` has
  no `is_term` method to fake (`grep -c 'is_term\|is_terminal'
  indicatif-0.18.3/src/term_like.rs` returns 0). The other half, "otto can
  supply correct `width`, `height`, and the cursor operations for a stream that
  is not a terminal", failed on measurement:
  - `TIOCGWINSZ` on a non-terminal fd returns -1, `Term::size_checked()` is
    `None`, and `Term::size()` invents `(24, 80)`. Rows would be truncated to
    `width - 1` against a fabricated width.
  - Cursor ops into a pipe are only more bytes; nothing is erased. 12 redraws
    of ONE row wrote 181 bytes after the first frame, 2413 total, 26 copies of
    the row text.
  - `term_like` installs `rate_limiter: None` (`draw_target.rs:84-93`): 101
    frames for 100 ticks, against 20 through `term_like_with_hz(_, 1)`. If
    `always` is ever revisited, the constructor is `term_like_with_hz`, never
    bare `term_like`.
  The flag ships `auto | never`.
- **`OTTO_PROGRESS` is still stripped from the child environment at spawn, on a
  DIFFERENT rationale than the one it shipped with.** The original reason was
  that `OTTO_PROGRESS=always` propagates and restores the duplicate-renderer
  defect; with `always` cut, that reason is dead. The strip stays because
  `scheduler/task_execution.rs:223` inherits the parent env by design (its own
  comment: "Inherit current environment by default (no env_clear())"), so a
  parent's `OTTO_PROGRESS=never` would otherwise mute a nested otto that should
  be making its own `auto` decision. **Precisely: this changes behaviour only
  for a nested otto under a `tty:` task**, which is the only child that
  inherits the terminal and could therefore have resolved `auto` to Live. A
  nested otto under an ordinary task has its stderr captured by the parent, so
  its `auto` resolves Quiet whether the variable reaches it or not. The strip
  is still right unconditionally; its EFFECT is `tty:`-only, and an earlier
  revision stated the benefit generally (implementation audit round 1, Q2). Stripping one env var otto itself sets is
  NOT the nesting handshake this design rejects: no marker, no detection, no
  second source of truth.
- `never`: Quiet unconditionally. The `TURBO_UI=false` escape hatch, which
  exists because CircleCI allocates a tty and Turborepo's TUI crashes there.

**The facade.** Reached through a `OnceLock`, mirroring the `static`
`TERMINAL_LOCK` it replaces, so no signature threading through the scheduler.

```rust
impl Facade {
    /// One write. Ordered against every other facade write.
    pub fn write(&self, stream: Stream, bytes: &str);
    /// A contiguous run that no other writer may split: buffered-foreach
    /// replay holds ONE of these for a whole block, never one per line.
    /// The closure gets BOTH streams, not one writer: replay interleaves a
    /// subtask's stdout and stderr inside a single locked block
    /// (`scheduler/replay.rs:388-444`), so a `&mut dyn Write` cannot express it.
    pub fn block<R>(&self, f: impl FnOnce(&mut BlockStreams<'_>) -> R) -> R;
    /// Live-path only. No-op in Quiet. On `Facade`, NOT on `BlockStreams`:
    /// row mutation must happen outside a block, per the third rule below.
    pub fn task_started(&self, name: &str);
    pub fn task_finished(&self, name: &str);
}

/// Both streams, locked together, handed to a `block` closure. Replay
/// interleaves stdout and stderr under ONE lock with per-stream flushes
/// (`scheduler/replay.rs:388-444`), so a single `&mut dyn Write` cannot
/// express it and calling back into the facade would break the no-reentry
/// rule.
pub struct BlockStreams<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}
```

Three rules the facade enforces so callers cannot break indicatif's lock order
(indicatif is internally `BarState` -> `MultiState`; `block` holds
`MultiState` for the whole closure):

- nothing inside a `block` closure may touch a `ProgressBar`. That is
  `MultiState` -> `BarState`, an inversion against the ticker, and it
  self-deadlocks anyway because the lock is not reentrant.
- nothing inside a `block` closure may re-enter the facade.
- all row mutation (`task_started` | `task_finished`) happens outside a block.

The facade is the only thing that knows these rules exist. That is the point:
the shipped doc rejected the spinner partly because an erase-before-write
invariant would bind "every current and future terminal writer". Behind a
facade it binds ONE module.

**Row cap.** `K = max(3, term_height - 6)`, leaving room for the overflow row
plus the shell prompt. indicatif silently drops rows past `term.height()`, so
without an explicit cap the row it drops is the `... and N more running` line
that exists to tell you rows were dropped.

**Migration.** `OttoSpec` is `deny_unknown_fields` (`src/cfg/otto.rs:268`), so
`progress-interval` cannot merely be dropped: existing ottofiles would fail to
load. `progress-interval` is retained as a deprecated key for one release,
accepted and ignored with a warning to stderr, then removed.

### Lifecycle and teardown

The region must be erased on every exit path, not just the happy one. indicatif
sets no terminal mode, so a hard kill leaves at most a half-drawn row and never
a broken terminal; that is why this design does not need the signal machinery
Alternative 2 would. It still needs all four of these:

**`Drop` is not the mechanism.** The facade lives in a `OnceLock`, and a
`OnceLock` static never runs `Drop`. Two exit paths also bypass unwinding
entirely: `app.rs:597` and `main.rs:236`. Teardown is an explicit call, not a
destructor, and its ORDER matters: at `main.rs:235-236` the sequence is
`report_fatal(&e);` then `exit(1)`, so teardown must run BEFORE `report_fatal`
at `:235`. Clearing the region after the fatal error is printed erases the
error.

Two precisions an earlier draft got wrong. `auto_prune` (`app.rs:507`) is an
ordinary awaited call, NOT an unwinding bypass; it matters only because it can
`eprintln!` (`pruning.rs:176-177`), so teardown must precede it. And
`before_exit` is NOT the first-signal path: `install_stop_handler` reaches it
only on a SECOND signal (`app.rs:585-597`), so first-signal cancellation needs
its own teardown call.

The seam already exists. `install_stop_handler(cancel, on_first, before_exit)`
(`app.rs:579-599`) was built so the `--tui` path could "hand back a terminal"
before a second-signal exit. Copy it rather than invent:

- **normal exit**: explicit `facade.teardown()` on every return path, ahead of
  `auto_prune` (`app.rs:507`).
- **first SIGINT**: the run cancels and prints a run-cancelled message
  (`flush_cancelled_groups`). `before_exit` does NOT fire here, so this path
  needs its own teardown call, placed so the message is not written into rows
  about to be erased.
  **This one is Phase 5's, not Phase 2's.** Phase 2 shipped three of these four
  sites deliberately. On the plain path a first signal already reaches teardown
  through the normal return into `auto_prune`, so nothing is unguarded today;
  what this bullet asks for is an ORDERING against rows, and there are no rows
  until Phase 5. Phase 5 adds it.
- **second SIGINT**: `before_exit` clears the region, matching what `--tui`
  already does with it, ahead of the `eprintln!` at `app.rs:596`.
- **fatal error**: teardown ahead of `report_fatal` at `main.rs:235`.
- **panic**: **no panic hook.** A hook that clears the region would call into
  indicatif, whose `MultiState` write lock (`multi.rs:193`, `:389-391`) is
  likely already held by the panicking thread, and whose `state.write().unwrap()`
  does not recover poisoning the way `TERMINAL_LOCK` does (`output.rs:48-49`).
  The hook would deadlock or double-panic. We rely instead on the property that
  chose indicatif: it sets no terminal mode, so a panic leaves at most a
  half-drawn row and never a broken terminal.
  **Scope this honestly.** "The facade catches and degrades" covers panics
  inside calls the facade makes. It does NOT cover indicatif's own ticker,
  which runs on a thread indicatif spawns (`progress_bar.rs:706`) and locks and
  ticks (`:729-733`) outside any facade call. A panic there poisons the bar
  lock with no facade frame on the stack. The facade must therefore treat a
  poisoned bar lock as "renderer permanently retired" on its next call, not as
  something it can catch at the moment it happens.
  **The panic boundary itself was missing and is now `Region::shielded`**
  (implementation audit round 1, finding M3), wrapping `suspend`, `started`,
  `finished`, `refresh`, `erase` and `rearm`. It is needed because 0.18.3's
  `MultiState::suspend` is `self.clear(now).unwrap(); f(); self.draw(..).unwrap()`,
  so a terminal write error inside a suspension panics in exactly the case this
  section scopes as covered. `suspend` takes its closure out of an `Option`
  inside the suspension, so the wrapped write happens EXACTLY ONCE whichever of
  the two `unwrap`s trips.
  **Retiring the renderer deliberately LEAKS the bars** (`std::mem::forget`).
  `BarState::drop` draws through the very lock the panic poisoned, and a
  destructor panic during unwinding aborts the process, which would turn "the
  renderer degraded" into "otto died at teardown". One region exists per run
  and holds one row per running task, so the leak is a handful of allocations
  at the end of a run. This is load-bearing, not an oversight.
  **Update, implementation audit round 1: the poisoned-bar-lock half of this is
  MOOT BY CONSTRUCTION and is deliberately not implemented.** The hazard was
  indicatif's own ticker thread (`progress_bar.rs:706`) locking and ticking
  outside any facade call. Phase 5 never arms that ticker: otto drives its own
  redraw through `Facade::refresh`, because a steady tick would redraw a
  message nothing recomputed and the duration would never advance. With no
  indicatif-spawned thread there is no frame that can poison the bar lock
  outside a facade call. The OTHER half, a panic boundary around calls the
  facade itself makes, was genuinely missing and is finding M3.
- **terminal hangup**: writes fail `EIO`. Best effort, never fatal, matching
  `report_fatal` (`main.rs:240-249`).

**Unterminated final chunk. This is a defect on `main` TODAY, not a hazard the
region introduces.** `read_until(b'\n')` yields a final chunk with no trailing
newline at EOF (`printf DONE`). Phase 0 measured it on `1783f1c` with no region
anywhere in the tree: otto's own completion line is concatenated onto the
task's last line, observed as `[nonl] DONE[nonl] finished successfully`. An
earlier draft framed it as a renderer precaution, which is exactly why no phase
owned it and it would have shipped unfixed.

**Phase 5 owns the fix** (decided 2026-09-16, see Resolved Decisions). The
facade tracks whether the last byte it wrote was `\n` and emits one before
drawing rows OR printing a completion line.
**Per STREAM, not per process. One flag for two streams is wrong and shipped
wrong** (implementation audit round 1, finding M1). With a single flag, an
unterminated chunk on stderr put a leading newline into a captured stdout that
never asked for one, and an unterminated chunk on a redirected stdout was
answered by a newline sent to a stderr somewhere else entirely, so the original
`printf DONE` defect still reproduced whenever the two streams had different
destinations. The remediation keeps `at_line_start` per stream and re-couples
them only when they genuinely share one destination, decided once from `fstat`
by comparing `(st_dev, st_ino, st_rdev)` on fds 1 and 2. That is a property of
how otto was invoked and cannot change mid-run, so it is measured at startup
and not re-probed. The pre-row-draw newline goes to the stream the rows land
on, never to whichever stream the last write happened to use. Without it the region additionally
overwrites the child's last line and a later erase deletes child output. Replay
already does this explicitly (`scheduler/replay.rs:347-352`); the live path and
the completion line must too.

**Phase 2 deliberately does NOT fix it.** Phase 2's criterion is output
byte-identical to Phase 1's tree, and this fix changes bytes by construction.
Fixing it there would mean relaxing the one assert that catches a facade
refactor silently reordering output.

### Implementation Plan

**Every `file:line` in this plan is anchored to `main` at `1783f1c`**, the
commit the whole doc was written and measured against. The phases move code, so
these numbers drift as they land: Phase 1 removed 26 lines from `app.rs`, which
shifted `:507` to `:481`, `:596` to `:570` and `:597` to `:571`. Verified by
Phase 2, which checked every citation it relied on against both trees. Treat a
citation as a pointer to a named thing, not to a line in your working copy, and
re-locate by symbol.

#### Phase 0: PTY spike, zero production code
**Model:** opus
- Prove indicatif 0.18.3 behaviour against otto's real output shape in a pty
  harness: the late-parent-report / tty-child schedule, buffered replay, a
  final chunk with no trailing newline (`printf DONE`), SIGWINCH, terminal
  hangup, SIGINT.
- Prove `always`: `ProgressDrawTarget::term_like` draws through a pipe because
  its branch (`draw_target.rs:198`) has no terminal predicate, unlike
  `TargetKind::Term` at `:176`. Nothing needs to claim to be a terminal;
  `TermLike` has no such method. What must be proven is that otto can supply
  correct `width`, `height`, and cursor operations for a non-terminal stream.
  If it cannot, `always` is cut and the flag ships as `auto | never`.
- Decide from measurement whether to stay on 0.18.3 or bump. 0.18.3 checks
  `term.is_term()` only, with NO dumb-terminal check; 0.18.6 adds `is_dumb()`.
  If we stay, otto implements the `TERM=dumb` policy itself.
- **Success criteria:** a committed pty test that FAILS on today's binary and
  names the handoff race; a recorded verdict on `always` and on the indicatif
  version, each with the measurement that chose it.

#### Phase 1: Remove the heartbeat.
**Model:** sonnet
- Delete `beat_line` and the ticker emit path, including its terminal write at
  `heartbeat.rs:429`. Keep `TaskClock` whole; rename the module `clocks.rs`.
- Ordered FIRST, not last. `rg -n 'terminal_lock\(\)' src/` shows **four**
  acquisition sites, and `heartbeat.rs:429` is one of them. Deleting
  `TERMINAL_LOCK` while the heartbeat still holds it is not a no-behaviour-change
  refactor, which is what the previous ordering claimed.
- **Success criteria:** `rg -c 'still running \(' src/ tests/` gives empty
  output (exit 1);
  `rg -n 'let _terminal = terminal_lock\(\)' src/` returns exactly three.
  That spelling on purpose: bare `terminal_lock()` also matches the definition
  at `output.rs:48`, so "three acquisition sites" and its own command
  disagreed. Observed on main: four.

#### Phase 2: The output facade. No behaviour change, with one stated exception.
**Model:** opus
- Introduce `progress::facade` over the three remaining acquisition sites:
  `TeeWriter::write` (`output.rs:198`), `scheduler/replay.rs:388`,
  `scheduler/replay.rs:479`.
- Delete `TERMINAL_LOCK`. The facade holds the ordering invariant and keeps
  poison recovery (`output.rs:48-49`), which indicatif does not.
- Add `facade.teardown()` to every exit path, BEFORE the thing that writes:
  ahead of `auto_prune` (`app.rs:507`), ahead of `report_fatal`
  (`main.rs:235`), inside the `before_exit` closure of `install_stop_handler`
  which already runs ahead of the second-signal `eprintln!` (`app.rs:595-596`).
- **The one intended behaviour change:** facade writes are best-effort and
  never panic, where `print!` / `eprint!` panicked on a write error. This is the
  Lifecycle section's own "terminal hangup: best effort, never fatal" contract
  arriving a phase early, and it is invisible to the byte-identical criterion
  because that criterion compares successful runs. Called out so the phase
  heading is not a lie.
- **Success criteria:** the writer grep in Acceptance Criteria gives empty
  output. One spelling, defined once there, referenced here. Full suite green
  with output byte-identical to Phase 1's tree.

#### Phase 3: Mode plumbing and config migration.
**Model:** sonnet
- `--progress`, `OTTO_PROGRESS`, `otto.progress`. Precedence flag > env > file.
- Strip `OTTO_PROGRESS` from the child environment at
  `scheduler/task_execution.rs:223`.
- `progress-interval` accepted, ignored, warned on. Update its help text at
  `src/cli/parser/help.rs:72`, which promises heartbeat behaviour that Phase 1
  already removed.
- **`otto.progress` is a raw `Option<String>` in `OttoSpec`, not a typed enum.**
  `executor` already depends on `cfg`, so a typed field would cycle the module
  graph. The value is validated through the same accepted-values list as the
  CLI flag, at `Parser::parse()` time, so there is still one source of truth for
  what `auto | never` means.
- **`docs/commands/ottofile-reference.md` is updated HERE, not in Phase 5.**
  Phase 5 owns README and the rest of `docs/commands/`, but the reference page
  is cross-checked by an exhaustive-key test that destructures `OttoSpec`, so
  adding `otto.progress` fails to compile until the page and the test move
  together. Not optional and not deferrable.
- **Success criteria:** an ottofile carrying `progress-interval` loads and
  warns; a nested otto sees no `OTTO_PROGRESS` in its environment.

#### Phase 4: Terminal-ownership gate. No renderer yet.
**Model:** opus
- **Ordered BEFORE the region on purpose.** The previous draft shipped the
  region in Phase 4 and the handoff in Phase 5, which leaves an intermediate
  tree worse than today: a `tty:` task inherits the terminal
  (`scheduler/task_execution.rs:298-299`) while the ticker draws, with nothing
  gating it. Ordinary tasks do not block a tty admission at all
  (`scheduler.rs:342`, `Admission::Tty => in_flight.exempt == 0`).
- Extend the admission rule so no `tty:` task is admitted while ANY admitted
  task has not yet been seen report. The counter already exists in the right
  shape: `in_flight_len` (`scheduler.rs:408`) covers every admitted task until
  `reported` (`scheduler.rs:484`). `InFlight` itself (`scheduler.rs:317-321`)
  holds only `tty` and `exempt` and records no completion stage, so an earlier
  draft's "completed-but-unprinted report" predicate was not expressible; this
  one is, with no new accounting.
- **A bare predicate is not enough: it lets ordinary tasks overtake the tty
  task.** The launch loop defers and continues in the same pass
  (`scheduler.rs:1638-1639`, `deferred_by_admission.push(task); continue;`),
  ordinary tasks are ungated (`scheduler.rs:348`, `Admission::Capped => true`),
  and the deferred tty task is restored to the head (`scheduler.rs:1662-1663`)
  only to be deferred again. With one active ordinary task and a ready queue of
  `[tty, long-1, long-2]`, the gate admits both long tasks ahead of the tty
  task it just deferred.
  Severity, stated precisely: this is **not** a deadlock. The task set is
  finite and admitted tasks terminate. It is unbounded-in-practice delay, where
  one long-running ordinary task holds the tty task off for its whole duration.
  Today's rule does not have that property, so shipping the bare predicate
  would be a regression.
  The fix is a **drain-mode flag on the loop**: once a `tty:` task is deferred
  for ownership, the loop stops admitting ordinary tasks until it drains and
  the tty task goes in. Not a stronger predicate.
  **Drain mode covers `Admission::Capped` only** (this doc's own wording is
  "stops admitting ordinary tasks"). Phase 4 measured the alternative:
  extending drain to exempt items passed both success criteria and broke
  `a_tty_task_and_an_exempt_group_never_overlap_at_one_job`, where one item of
  a `jobs: all` group started and the other two were deferred.
- **The tty gate must YIELD to a ready bookkeeping parent.** Not in the
  original draft, and the doc's own Phase 0 criterion still fails without it:
  a virtual foreach parent (empty action) becomes ready only once every subtask
  has reported, so at that instant the in-flight total is 0 and a deferred
  `tty:` task sitting at the queue head is admitted ahead of the group's own
  completion line (measured: `OWNER-START` at 6004, `[bulk] finished
  successfully` at 6005). The gate therefore recognises a bookkeeping item and
  lets it through ahead of the tty task.
- **A second `tty:` task is refused while one is admitted.** The in-flight
  total subsumes the old `tty` counter, so this falls out of the same
  predicate rather than needing its own.
- Ownership acquire/release around every `tty:` spawn, with buffering of otto
  writes while surrendered. The buffer is BOUNDED, because buffering a whole
  late replay as bytes would replace replay's 64 KiB streaming bound
  (`scheduler/replay.rs:16`) with full log size during a long interactive
  child. It does NOT spill to the streaming path: that path writes the real
  stdout/stderr handles, and writing them while a `tty:` child owns the
  terminal is the exact collision the surrender exists to prevent. (Address
  note: this was `scheduler/replay.rs:389-391` at `1783f1c`; Phase 2 moved the
  handles inside `Facade::block`, called from `replay.rs:393`. The claim holds,
  the old line range does not.) Past the cap, a surrendered replay spills to a
  file and is re-streamed from there on re-arm, which is the one destination
  already guaranteed not to touch the terminal.
  **The spill file is `<run>/tasks/<tty task>/otto-surrendered.log`**, not the
  replayed subtask's own log: the facade has no task identity at the point it
  holds bytes, so it cannot address the originating task's file. Bounds and the
  no-terminal-contact guarantee are unchanged by that choice.
- Cover spawn failure and cancellation.
- **Success criteria:** a test admitting a `tty:` task while an ordinary task
  is mid-report FAILS on today's binary and passes here; a test with ready
  queue `[tty, long-1, long-2]` and one active ordinary task asserts the tty
  task is admitted at the next replenishment, NOT after `long-1` and `long-2`
  finish. (No `terminal_lock` clause here: Phase 2 already deleted it, so the
  Phase 1 wording carried over by the swap would assert something impossible.)

#### Phase 5: The live region, and the completion line.
**Model:** opus
- `MultiProgress` behind the facade. One row per running task. `{wide_msg}`,
  not `{msg}`. Rows truncated to `width - 1`, sanitized (strip C0, expand tabs,
  cut on grapheme boundary, only otto-generated SGR).
- Bounded rows: K rows plus one persistent `... and N more running`.
- **Set row styles explicitly for stderr. Do NOT rely on indicatif's
  auto-targeting.** On 0.18.3 `ProgressDrawTarget::is_stderr()` returns `false`
  for `TargetKind::Multi` (`draw_target.rs:133-140`; fixed at `:150` in
  0.18.6), and `ProgressBar::set_style` is its only consumer
  (`progress_bar.rs:161`). Left alone, every row in the region is styled with
  console's STDOUT colour decision while the region draws on STDERR. Same
  defect class otto fixed at `output.rs:192-194` in `47b4605`. Found by
  Phase 0; it is the standing cost of staying on 0.18.3.
- `mp.remove(&pb)` explicitly on completion. indicatif's `mark_zombie` reaps
  only the leading run, so a task finishing third leaves a dead row.
- Rows read duration from the `TaskClock` they hold
  (`scheduler/task_execution.rs:354`), never from `task_start_times`.
- Explicit region erase on surrender, driven by Phase 4's gate.
  `set_draw_target(hidden())` does NOT erase: `disconnect` on a terminal
  target is a no-op.
- **Completion line uses `TaskClock`, not `task_start_times`.** An earlier
  draft called this formatting rather than measurement. That was wrong.
  `scheduler.rs:1524-1530` stamps every task with one shared run-start
  `Instant` before dependency waits ("Tasks are conceptually 'in progress'
  while waiting for deps"), while `TaskClock` starts at stream creation.
  (That range is `:1531-1541` as of Phase 4; re-locate by symbol, do not trust
  the number.) A
  task that waits 10m on deps and runs 1s would show `running 1s` in its live
  row and `10m01s` on its completion line, breaking the one goal this design
  exists to serve. Dep-wait time is a different number and is out of scope.
- **Fix the unterminated-final-chunk concatenation** (see Lifecycle and
  teardown). Pre-existing on `main`, measured by Phase 0, owned here. The
  facade emits a `\n` before a completion line or a row draw when the last byte
  it wrote was not one.
- Flip `docs/design/2026-09-15-idle-task-heartbeat.md` to `Superseded by ...`.
  Update README and `docs/commands/`, EXCEPT
  `docs/commands/ottofile-reference.md`, which Phase 3 already updated because
  an exhaustive-key test forced it. README still shows `--progress-interval` as
  live and has no `--progress` row; that is this phase's to fix.
- **Success criteria:** a pty test asserting screen CONTENTS after a chatty
  task plus a silent task; zero RENDERER bytes under `--progress never`, with
  the same test asserting task output and completion lines still arrive; a
  fixture with a task that WAITS on a dependency for **at least 5s**,
  asserting its live row and its completion line agree within 1s. The wait must
  exceed the tolerance or the fixture cannot fail: a 0.2s wait plus a 0.1s row
  yields a wrong completion of 0.3s, which passes a 1s tolerance on the old
  measurement. Plus a `printf DONE` fixture (final chunk, no trailing newline)
  asserting the completion line starts on its OWN line, under `--progress auto`
  on a pty AND under `--progress never` through a pipe. It must FAIL on
  `1783f1c`, where Phase 0 recorded `[nonl] DONE[nonl] finished successfully`;
  a fixture that passes on main is not testing this fix.

## Acceptance Criteria

Every criterion below was RUN against `main` at `1783f1c` on 2026-09-16 and its
output recorded. A criterion whose command cannot run until a phase ships says
so.

Pass condition for every `rg` criterion below is **empty output, exit 1**.
`rg -c` prints per-file counts and never prints a bare `0`, so "returns zero"
was not a checkable statement.

- [ ] `rg -c 'still running \(' src/ tests/` gives empty output.
      **Observed on main:** `src/executor/heartbeat.rs:1`,
      `src/executor/heartbeat_tests.rs:5`, total 6 lines.
      Scoped to the emitted line SHAPE on purpose: bare `still running` stays
      nonzero legitimately (`src/tui/app.rs:201`,
      `src/cli/commands/upgrade_tests.rs:987`), so "zero occurrences of
      `still running`" would be unpassable by design.
- [ ] `rg -c 'TERMINAL_LOCK' src/` gives empty output.
      **Observed on main:** `src/executor/output.rs:2`,
      `src/executor/heartbeat.rs:3`, total 5 lines.
- [ ] `rg -n 'eprintln!|println!|eprint!|print!' src/executor/ -g '!*_tests*.rs' | rg -v 'progress/facade'`
      gives empty output. **This is the one canonical writer grep**; phases
      reference it rather than respelling it.
      **Observed on main: 7 matches** across four files: `output.rs:200`,
      `output.rs:202`, `graph.rs:109`, `pruning.rs:176`, `pruning.rs:177`,
      `scheduler/replay.rs:481`, `scheduler/replay.rs:484`. All seven are
      Phase 2's routing surface, including `graph.rs` and `pruning.rs`, which
      an earlier draft wrongly called "outside the pattern's reach".
      The glob is `!*_tests*.rs`, not `!*_tests.rs`: this repo splits tests as
      `scheduler_tests_b.rs`, which the narrower glob misses, and with it the
      literal command returns 9, two of them test-only `println!`s.
      `write_all` is deliberately NOT in the pattern: it returns 21 matches on
      main including `output.rs:182`, which writes the LOG FILE, not the
      terminal. An ownership bypass and an authorized non-terminal writer are
      different things and one grep cannot separate them.
      **Scope limit, stated so it is not mistaken for a whole-tree guarantee.**
      The grep covers `src/executor/` only, and raw macros outside it survive on
      purpose: `src/cli/parser.rs`, `src/main.rs` and `src/app.rs` hold 25
      between them as of Phase 3. They fall into two groups, and neither can
      collide with a region:
      - **Before the facade is live**: CLI parse errors, `--help`, and Phase 3's
        `progress-interval` deprecation warning (`parser.rs:1079`). All emitted
        at `Parser::parse()` time, before a run starts and before anything is
        drawn.
      - **After it is torn down**: the second-signal `eprintln!` in `app.rs` and
        the error prints reached from `main.rs`. Verified in the committed code:
        `before_exit()`, then `facade().teardown()`, then the `eprintln!`.
        Routing these through the facade would route them through a torn-down
        facade.
      The invariant is "the facade is the only writer WHILE it is live", not "no
      macro exists anywhere", and those two statements needed separating. An
      earlier revision of this bullet said "three raw macros", which was wrong
      by 22 and is the same stale-cross-reference class this doc keeps hitting.
- [ ] Under a pipe, `--progress auto` and `--progress never` produce
      byte-identical stderr.
      **Cannot run on main:** `--progress` does not exist until Phase 3.
      Today's equivalent, `OTTO_PROGRESS_INTERVAL=0` vs default, differs by one
      beat line per interval, which is the defect.
- [ ] A nested otto run (outer on a tty, inner spawned by a normal task) emits
      exactly one live region and zero duplicate rows.
      **Cannot run on main:** no renderer exists until Phase 5. Today's
      equivalent reproduces the defect: two beats per interval, verbatim in
      Ian's 2026-09-16 paste.
- [ ] An ottofile carrying `progress-interval` loads successfully and warns.
      **Cannot run on main:** on main the key is live and silent, not
      deprecated. `rg -c 'progress[-_]interval|PROGRESS_INTERVAL' src/ tests/`
      **observed 98 lines**, which is the Phase 3 removal surface.
      **After Phase 3 (`ef882bb`) it reads 84, and that is the intended end
      state, not a partial job.** The key is deprecated-and-warned for one
      release rather than removed, so its parser, its warning, and the tests
      asserting the warning text all legitimately survive. This count is
      recorded as a SURFACE, never as a pass/fail assert; the falsifiable
      criterion is the warning above.

## Resolved Decisions

- **2026-09-16, two renderers chosen by tty.** 7 of 7 surveyed tools do this:
  BuildKit (`--progress=auto` -> `tty|plain`), Bazel (`--curses`), Codex CLI
  (TUI | `codex exec`), Cargo, Ninja, Turborepo, Gemini/Ink. None animates into
  a pipe.
- **2026-09-16, no nesting detection.** No surveyed tool uses an env handshake
  to detect a parent instance of itself. `MAKELEVEL` is the only env precedent
  and it exists for makefile authors, not UI suppression. A marker would be a
  second source of truth that can diverge from the first, which is the npm bug
  (`5b858c6`) the shipped doc already cites.
- **2026-09-16, indicatif over ratatui Inline.** ratatui's inline viewport does
  `clear_region(ClearType::All)` on a horizontal shrink
  (`ratatui-core-0.1.2/src/terminal/resize.rs:41-45`, read from the published
  crate source; ratatui-core is NOT vendored in this repo so it is not locally
  verifiable), wiping child output already in scrollback. Narrowing a window would destroy task output.
- **2026-09-16, elapsed semantics kept, labeling fixed.** No surveyed tool
  displays idle duration as reassurance; every live display shows elapsed since
  the start of a NAMED subject, and idle appears only as a CI kill reason
  (Travis, CircleCI, both 10m). Bazel binds the number to its subject
  (`ActionExecutionStatusReporter.java`). The v2.5.3 defect was a bare
  parenthetical on a line that only appears during silence.

- **2026-09-16, panel round 1: the `always` semantics question resolved**
  (it was numbered OQ2 before the ottofile-key question took that slot).
  `always` genuinely overrides,
  matching cargo's `term.progress.when`, AND otto strips `OTTO_PROGRESS` from
  the child environment at `scheduler/task_execution.rs:223`. The seats split:
  the Architect wanted `always` renamed `force` or dropped, because
  `OTTO_PROGRESS=always` is inherited by a nested otto and bypasses the `auto`
  safeguard. That hazard is real and specific to the ENV form (`--progress
  always` on the CLI does not propagate; cargo carries the identical hazard
  with `CARGO_TERM_PROGRESS_WHEN`). Stripping the var is not the nesting
  handshake this design rejects: no marker, no detection, no second source of
  truth, so the npm `5b858c6` rationale does not bite. Phase 0 may still cut
  `always` if otto cannot supply width/height/cursor ops for a pipe.
- **2026-09-16, Phase 0 SUPERSEDES the entry above: `always` is CUT.** The
  conditional that entry ended on fired. The flag ships `auto | never`.
  Measurements in API Design and in
  `docs/design/2026-09-16-live-progress-renderer-implementation-notes.md`.
  The `OTTO_PROGRESS` strip survives the cut on the new rationale recorded in
  API Design, so Phase 3's bullet is unchanged in substance.
- **2026-09-16, CLOSED: Phase 5 owns the unterminated-final-chunk fix.** Raised
  by Phase 0, which measured the concatenation on `1783f1c` with no region in
  the tree, so it is a pre-existing defect that no phase owned. Three homes were
  weighed. Phase 2 is the causally correct owner, since the facade is the single
  writer, but its byte-identical criterion is the only assert guarding the
  refactor against silent output reordering and this fix changes bytes by
  construction. Filing it out of scope would flip this doc to `Implemented`
  while its own notes record a measured, unfixed defect. Phase 5 already owns
  the completion line by title, has the facade available, and pins no bytes
  against a prior tree, so it absorbs the fix at the cost of no existing
  criterion.
- **2026-09-16, panel round 1: OQ3 resolved, the facade is the right seam.**
  Both seats yes, independently. Neither argued for a simpler place.
- **2026-09-16, CLOSED: a captured run gets no periodic liveness (option A).**
  Recommended by the author, concurred independently by both panel seats, and
  across three rounds neither seat named a concrete flaw. Absent a named flaw
  the recommendation ships rather than becoming a question. Supporting
  evidence: 6 of 7 surveyed tools emit nothing periodic into a pipe, and piped
  `cargo` prints nothing for minutes during a long `rustc` without anyone
  treating it as a defect.
  **Consequence, accepted with open eyes:** `philo slim-dump` under
  `2>&1 | tee` is silent from its last output line to its completion line. The
  completion line carries elapsed, so the question "how long did it take" is
  answered; "is it alive right now" is not, in a captured stream.
  **Revisit condition:** someone reports a captured run they could not
  diagnose because of the silence. Option B (Bazel's bounded backed-off beat,
  10s -> 30s -> 60s, capped rows plus overflow) is then additive behind
  `--progress`, and nothing in this design forecloses it.
- **2026-09-16, CLOSED: the `otto.progress` ottofile key stays.** The Architect
  called it unrequested scope. Overruled on a standing rule: config drives
  behaviour or it does not exist, and tunables ship through the standard
  delivery path. `progress-interval` is ALREADY an ottofile key
  (`src/cfg/otto.rs:303-304`), so shipping the replacement without it would
  remove a project default ottofiles can set today, which is a regression in
  surface dressed as a simplification. The cost is a two-step ship order
  instead of one, which the deprecation of `progress-interval` forces anyway.

- **2026-09-17, implementation audit round 1 (post-Phase-5, pre-tag).** Run
  against `1783f1c..9c22b3d` with the doc at `Status: Implemented`, while every
  commit was still local. Verdict: do not tag. Five must-fix findings, three
  cheap wins, one deferred, every one reproduced by the panel against old and
  new binaries. Four were code (`at_line_start` shared across two streams; the
  row cap dropping the overflow row it exists to protect; no panic boundary
  around indicatif; no `\x1b[0m` on handoff reclaim) and one was this document
  (three statements false rather than stale, corrected in place above and each
  marked). Confirmed sound by the same audit: the bookkeeping-parent yield with
  no new overtaking hole, drain scoped to `Capped`, `Facade::refresh` over
  indicatif's ticker, per-row stderr styling, no surviving live `always` claim,
  and the single-writer invariant. Deferred: a partially written spill record
  replayed twice in part, which needs disk-full during a tty handoff and
  duplicates rather than loses bytes.

## Alternatives Considered

### Alternative 1: Fix the two defects in place
- **Description:** print idle instead of elapsed; propagate `OTTO_PROGRESS=0`
  to task children.
- **Why not chosen:** a third swing at a feature Ian asked to be easy, and it
  keeps the ungated line that produced both defects. Ian: "lol nuke it then".

### Alternative 2: DECSTBM scroll-region sticky footer
- **Description:** Codex's mechanism (`codex-rs/tui/src/insert_history.rs:343`,
  `\x1b[{t};{b}r`). apt does the same in `PackageManagerFancy`.
- **Pros:** stateless to re-arm after an uncontrolled writer. Survives
  `otto | tee` and `otto | less`.
- **Cons:** SIGWINCH re-arm mandatory (xterm resets margins on resize), signal
  teardown for SIGINT/TERM/HUP/QUIT, and frozen bottom rows after a hard kill.
  Does not protect the reserved rows anyway: `CUP` is not clamped by margins
  and `TIOCGWINSZ` still reports full height, which is why apt lies to dpkg's
  children via `ioctl(TIOCSWINSZ)`, something otto cannot do for an inheriting
  `tty:` task.
- **Why not chosen:** otto knows every writer it starts, so the handoff gives
  us the same safety with no terminal state to restore on SIGKILL.
- **Revisit condition:** `otto | tee` becomes a workflow someone actually runs
  interactively.

### Alternative 3: `/dev/tty`
- **Why not chosen:** rejected in the shipped doc and the reasoning holds, with
  a better argument available: a progress indicator is not a prompt.
  `2>/dev/null` is a legitimate mute the stderr gate honors for free. Every
  mainstream progress renderer (cargo, apt, git, npm, Bazel, BuildKit) stays on
  stderr; only prompts and full-screen pickers (fzf, sudo, ssh, gpg) reach for
  `/dev/tty`.

### Alternative 4: Alternate screen
- **Why not chosen:** mode 1049 clears on entry and `?1049l` restores the
  pre-run primary screen. The entire run's output would vanish on exit.

### Alternative 5: Revert and ship nothing

- **Description:** delete the heartbeat in a patch release, keep the two design
  docs as the writeup, add no renderer.
- **Pros:** it is literally what Ian said ("lol nuke it then ... if not easy
  not worth it"). Zero new machinery, zero new failure modes, and the defect
  class is closed permanently rather than replaced.
- **Cons:** the original request goes unanswered. `philo slim-dump` looks
  frozen again, which is the problem that started this.
- **Why not chosen:** the request is legitimate and the tty-gated shape is the
  industry-standard answer, not an invention. But this alternative is recorded
  rather than dismissed, because this design IS more machinery than the
  heartbeat was, and Ian's stated bar was "easy". If OQ1 resolves toward
  minimalism, this is the option that honours it.
- **Revisit condition:** Phase 0's spike shows the handoff cannot be closed
  cleanly. Then ship nothing rather than ship a third broken version.

## Technical Considerations

### Dependencies

`indicatif` and `console` are already in `Cargo.toml` and `indicatif` already
drives a bar in `src/cli/commands/upgrade.rs:4`. `Cargo.lock` pins indicatif
**0.18.3** and console **0.16.2**. **Phase 0 decided: STAY on 0.18.3.** Two
reasons, both standing: 0.18.6 requires console >= 0.16.4 against the locked
0.16.2, so it is a two-crate bump; and `auto` must read `CI` regardless, which
no indicatif version knows about, so otto computes the predicate either way.
otto owns the `TERM=dumb` policy itself. A Phase 0 test pins both locked
versions so a `cargo update` cannot invalidate the measurements silently. The
cost of staying is the Phase 5 row-style constraint.

**A third reason was given and it was FALSE. Corrected here rather than quietly
dropped** (implementation audit round 1, finding M5). This section previously
said 0.18.6 applies `is_dumb()` "only inside `ProgressDrawTarget::term()`, a
path this design does not take". The design takes exactly that path:
`region.rs:122` constructs the region with `ProgressDrawTarget::stderr()`,
which in 0.18.3 is literally `Self::term(Term::buffered_stderr(), 20)`
(`draw_target.rs:41-43`). So 0.18.6's `is_dumb()` WOULD apply to otto's target.
The conclusion survives on the two reasons above, and otto's own `TERM=dumb`
check makes the upstream one redundant rather than desirable, but the reasoning
was wrong and a later reader weighing the bump needs the corrected version.

### Performance

`mp.suspend` forces a redraw after every callback, bypassing indicatif's draw
throttle. Per-line suspension means each chatty line clears and redraws up to
K+1 rows. The facade batches: replay holds one suspension for a whole block,
not one per line. Phase 0 DID benchmark the chatty path
(`527d781`, `tests/progress_spike_indicatif_test.rs`), measuring BYTES WRITTEN
rather than wall-clock, because the claim above is about writes. Method and
numbers are in the Phase 0 implementation notes. K is chosen in Phase 5 against
them. The Phase 0 section's bullets never named this benchmark while this
section assigned it; that disagreement is resolved in favour of having run it.

### Security

None. No new inputs, no new network or filesystem surface. Child bytes written
to `stdout.log` / `stderr.log` remain verbatim and untouched, which is the
invariant Alternative 4 of the shipped doc protected.

### Testing Strategy

- pty harness asserting screen CONTENTS, not stripped text. `pty_stdout`
  (`tests/common/mod.rs:119-123`) strips CR but not cursor control, so
  escape-stripping cannot prove child output survived.
  **Known weakness in this criterion, stated rather than left to be
  rediscovered** (implementation audit round 1). Retaining the control bytes is
  necessary but not sufficient: a `contains` assertion over the captured stream
  cannot see an ERASURE, because the erased text is still present in the byte
  stream that produced the final screen. Proving "the user saw X" needs a
  terminal emulator replaying the stream into a screen buffer, which this
  design does not build. Phase 5's tests comply with this bullet literally and
  inherit its limit; that is a gap in the criterion, not a phase that skipped
  work.
- Negative cases that must bite: zero RENDERER bytes under `never` while task
  output and completion lines still arrive; no duplicate rows
  when nested; a dead row removed when a middle task finishes first.
- The Phase 0 race test must FAIL on today's binary.

### Cross-repo blast radius and ship order

otto is consumed by ottofiles across `tatari-tv`, so the schema is the blast
radius, not the renderer.

1. Ship the binary carrying `otto.progress` (accepted) AND `progress-interval`
   (deprecated, warned). No ottofile changes.
2. Only then may a shared ottofile adopt `otto.progress`.
3. Remove `progress-interval` a release later.

Skipping step 1 rejects every ottofile carrying the new key on any machine
still running an older otto, because `OttoSpec` is `deny_unknown_fields`. Order
is not a nicety here, it is the whole migration.

`philo` is the canonical consumer and the reporter's own repro, so it is the
verification target: `otto data` on a tty, then `2>&1 | tee`, then nested.

### Rollout Plan

Ship the binary before any ottofile adopts `otto.progress`. `progress-interval`
stays accepted-and-warned for one release so a rollback does not reject
existing ottofiles.

## Known Limits

- **Two ottos started independently in one terminal** (`otto a & otto b`) will
  both draw. Unfixable by any technique surveyed, including DECSTBM. Accepted.
- **A buffered foreach subtask that is newline-chatty into its buffer** resets
  its own idle clock while the terminal shows nothing, so its row can read
  `no output for 0s` while nothing has appeared. Carried over verbatim from the
  shipped design's Known Limits; the clock reset sits above the
  `suppress_terminal` branch (`output.rs:179` vs `:187`) and moving it would
  make every chattering buffered subtask look idle instead, which is worse.
- **`tty:` tasks get no progress at all.** They own the terminal by design.
- **Rollback.** `otto.progress` in a shared ottofile will be rejected by a
  binary predating Phase 3, because `OttoSpec` is `deny_unknown_fields`. Ship
  the binary before any shared ottofile adopts the key, and remove the key
  before rolling a binary back. This is the same hazard the shipped design
  carried for `progress-interval`.
- **`K = max(3, term_height - 6)`** is clamped to `term_height - 1` so a very
  short terminal cannot ask for more rows than exist.

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Handoff race survives into a `tty:` task | Med | High | Phase 0 test that fails first; Phase 4 gates tty admission on `in_flight_len` before any region exists |
| indicatif panic poisons the shared lock | Med | High | Facade catches and degrades to plain writes |
| Wrap-miscount drift on a narrow terminal | Med | Med | Truncate rows to `width - 1`; BuildKit's bargain |
| CI allocates a tty and we animate into its log | Low | Med | `auto` also checks `CI`; `never` is the documented escape |
| Facade refactor changes output ordering | Med | High | Phase 2 asserts byte-identical output vs Phase 1's tree, NOT vs `main`: Phase 1 deliberately changes output by removing the beat |

## Open Questions

None. The two original questions were closed 2026-09-16. Phase 0 then
raised two more and both are closed: the `always` cut and the owner of the
unterminated-final-chunk fix. All four are in Resolved Decisions.

## References

- Slack thread, DM `D084N7HV19S`, 2026-09-15..16
- `docs/design/2026-09-15-idle-task-heartbeat.md` (superseded by this doc)
- `docs/design/2026-09-15-idle-indicator-comparison.md`
- `docs/design/2026-08-31-buffered-foreach-computed-envs-required-params.md`:
  the terminal-lock design this revises
- `moby/buildkit` `util/progress/progressui/{display,printer}.go`
- `bazelbuild/bazel` `ActionExecutionStatusReporter.java`; issues 9170, 16119
- `openai/codex` `codex-rs/tui/src/insert_history.rs`
- `rust-lang/cargo` `src/cargo/util/progress.rs`; `cargo#4156`
- `npm/cli` commit `5b858c6`
