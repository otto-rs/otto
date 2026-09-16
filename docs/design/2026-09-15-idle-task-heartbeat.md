# Design Document: Idle Task Heartbeat

**Author:** Scott Idler
**Date:** 2026-09-15
**Status:** Implemented
**Review Passes Completed:** 5/5, plus review-panel round 1 folded in

## Summary

otto goes silent while a task works, and a user cannot tell a slow task from a
hung one. This adds a periodic `[task] still running (2m14s)` line on stderr,
emitted only after a task has produced no line for N seconds. It is an ordinary
newline-terminated line: no carriage return, no cursor control, no erase, no tty
predicate deciding whether it appears at all.

**What "still running" means here, one definition used everywhere:** a task with
a live child process that has produced no line of output for N seconds.
Liveness comes from the child registry; silence comes from the drain. Neither is
"nothing reached the terminal", which is a different question this design does
not answer.

## Problem Statement

### Background

- Requested by Ian McEachern in DM, 2026-09-15.
- Motivating case: `philo slim-dump`. The underlying tool renders a download
  progress bar when it owns a tty. Under otto the child gets a pipe
  (`src/executor/scheduler/task_execution.rs:325-326`), so the bar never renders
  and otto prints nothing for minutes.
- otto already injects its own non-child lines into a run: `[task] finished
  successfully` on stdout (`src/executor/scheduler.rs:1683`), failures on stderr
  (`:1752`). Adding otto-authored lines mid-run is established, not new.

### Problem

A silent task is indistinguishable from a hung task.

> ideally otto knows when an action is in flight and nothing is being printed,
> and could do a little ascii spinner to let the user know that it isn't just
> frozen

And the constraint that decides the shape, raised by Scott and accepted by Ian:

> if youre piping its output to a log, the spinner will spam the log with each
> `|` `/` `--` `\` ?
>
> Ideally it wouldn't. If there's no way to show the spinner without defacing
> logs then it's not worth the visual indication

### Goals

- A user watching a silent task learns, within a bounded interval, that otto is
  still working and how long that task has been running. (Ian)
- Captured output is never defaced: no carriage return, no cursor control, no
  partially overwritten or re-drawn line, in any capture route. (Scott, accepted
  by Ian)
- The feature introduces no new escape bytes. It does not make otto's captured
  output escape-free, because it already is not: `colored` decides colour from
  `stdout().is_terminal()` (`colored-3.1.1/src/control.rs:107`) and otto applies
  that decision to stderr too, so `otto task 2>log` with stdout on a terminal
  already writes a coloured `[boom] failed` into `log` today. Measured, see
  Acceptance Criteria. The heartbeat therefore builds its label uncoloured
  whenever stderr is not a terminal, rather than inheriting that behaviour.
- A task that produces output regularly emits nothing extra. (Ian: "if a process
  is running and there hasn't been a line logged in the last 5 (3?) seconds")
- The interval is configurable through the ottofile, per otto's `jobs`
  precedent. (`taste.md`: tunables through the standard delivery path)

### Non-Goals

- **An animated ANSI spinner.** Excluded. Revisit condition in "Alternatives
  Considered".
- **Per-task progress percentages or download bars.** otto cannot know a child's
  progress; only the child can.
- **Coverage of `tty: true` tasks.** Those inherit stdio
  (`task_execution.rs:297-298`), so their bytes never pass through otto. otto
  cannot tell a silent `tty:` task from a busy one, and guessing would print
  over an interactive prompt. Excluded on a correctness ground, not a scope one.
- **Changing what a child sees.** No pty allocation for non-`tty:` tasks. The
  session isolation at `task_execution.rs:273-285` stands.
- **TUI changes.** `--tui` owns its rendering; the existing `tui_mode` guard
  (`src/executor/scheduler/replay.rs:457`) suppresses this too.
- **Windows verification.** otto's terminal discipline is unix-shaped. The
  heartbeat adds no new platform surface, and no Windows claim is made.

## Proposed Solution

### Overview

Three parts, each landing on an existing otto seam:

1. **A per-task idle clock**, reset whenever that task produces a line of
   output.
2. **A liveness read**, taken straight from otto's existing `LiveChildren` map
   at tick time. Nothing new is maintained: that map is the only thing in otto
   that means "a process exists right now".
3. **A ticker thread** that wakes on an interval and, for each live task whose
   idle clock exceeds the threshold, emits one line under the process-wide
   terminal lock.

Output shape, honouring `--no-prefix` exactly as otto's status lines do, and
coloured only when stderr is itself a terminal:

```
[philo] still running (2m14s)
```

### Architecture

| Concern | Where it lands | Why there |
|---|---|---|
| idle clock | top of `TeeWriter::write`, `src/executor/output.rs:148` | every LINE a task produces passes through here exactly once |
| liveness | read `LiveChildren` directly via `blocking_lock()`, `src/executor/scheduler.rs:250` | no mirror to keep in sync, so it cannot drift |
| emission | a new SYNCHRONOUS function in `replay.rs`, taking `terminal_lock()` itself | `report_status_line` is `async fn` (`replay.rs:435`) and a sync ticker cannot call it |
| ticker | dedicated OS thread, outside the tokio runtime | `blocking_lock()` and `std::sync::Mutex` are both illegal from inside a runtime context |

**Why the clock resets in `TeeWriter::write`, not at the `terminal_lock()`
sites.** The question is "did this task produce a line", not "did bytes reach
the terminal". The drain reads `read_until(b'\n')` (`src/executor/output.rs:250`),
so `TeeWriter::write` is called once per newline-terminated line, not per byte.
That is load-bearing rather than incidental: a tool rendering a `\r`-redrawn
progress bar emits no newline at all, so it never calls `TeeWriter::write`, the
clock never resets, and the task is correctly heartbeated. `philo slim-dump`,
the motivating case, is exactly that shape. A buffered foreach subtask has
`suppress_terminal` set
(`output.rs:155`) and reaches the terminal only through ordered replay, so a
clock keyed on terminal writes would report a chattering subtask as idle and
heartbeat it continuously. The reset goes above that branch, at
`output.rs:148`, where the file write already happens unconditionally. The
scheduler's own status lines do not reset any task's clock, which is correct: a
sibling finishing says nothing about whether this task is stuck.

**Why `register_child`, not `ActiveTasks`.** otto has two in-flight ledgers.
`ActiveTasks::spawn` (`src/executor/scheduler.rs:460`) inserts *before* the body
acquires its semaphore permit, deliberately, with a regression test named for it
(`spawn_counts_a_task_in_flight_before_its_body_acquires_a_permit`).
`Admission::Exempt` tasks (a foreach group that declared `jobs:`) are launched
unbounded and throttled inside the body (`task_execution.rs:51-54`), so on a
`foreach ... jobs: 4` over 40 items `ActiveTasks` holds 40 while 4 processes
exist. A heartbeat off that ledger would name 36 queued items as running.
`register_child` / `deregister_child` bracket the actual `spawn()` / `wait()`
pair, so they cannot lie.

**Why the ticker reads `LiveChildren` directly rather than mirroring it.**
`LiveChildren` is `Arc<tokio::sync::Mutex<HashMap<String, ChildHandle>>>`
(`src/executor/scheduler.rs:250`). `tokio::sync::Mutex::blocking_lock` is legal
from a dedicated OS thread that is not inside a runtime context, so no mirror is
needed. This is not a convenience: a mirror would have to be updated at six
sites, not the five that are obvious. Beyond `task_execution.rs:304`, `:306`,
`:333`, `:378` and the `:474` failure backstop, `ActiveTasks::abort_all` clears
the whole map at `scheduler.rs:516` (called from `abandon_run`,
`support.rs:220`) on cancellation, and an aborted future never reaches `:474`.
A mirror that missed that site would heartbeat tasks SIGKILLed by Ctrl-C, for
the rest of the run. Reading the source of truth makes that failure impossible
to write.

The ticker thread must therefore be spawned with `std::thread`, never
`tokio::task::spawn_blocking`: `blocking_lock` panics inside a runtime context.

**Why an OS thread.** otto already steps out of async for this exact reason:
replay does its locked terminal work inside `spawn_blocking`
(`src/executor/scheduler/replay.rs:369-374`) because a `std::sync` mutex must
not be held across an `.await`. A tokio-task ticker would reintroduce the hazard
the existing code avoids on purpose.

**Buffered foreach subtasks are heartbeated, and the line appears outside the
block.** That is deliberate. The heartbeat is otto's progress notice, not the
subtask's output, so it never enters the replay buffer and never appears in
`stdout.log` / `stderr.log`. Taking `terminal_lock()` means it can never split a
replayed block.

**No bounded-latency promise is made.** `write_replay_blocks` takes the lock at
`replay.rs:374` and then iterates `for block in blocks` at `:388`, so it holds
it for a whole BATCH, not one block. The doc comment at `output.rs:34` says
"one whole block" and is imprecise. A heartbeat can therefore be delayed by an
arbitrarily large replay, which is acceptable (a replay is output, so the run is
visibly not frozen) but must not be written up as a guaranteed interval.

### Data Model

Liveness is read from `LiveChildren`, so the only new state is the clock. It is
keyed by task name and may legitimately outlive the child: an entry for a task
absent from `LiveChildren` is never read, so a stale entry is inert rather than
a bug.

```rust
/// Per-task timing. NOT a liveness ledger: LiveChildren is that, and
/// duplicating it is how a mirror drifts (see abort_all, scheduler.rs:516).
struct TaskClock {
    /// For the elapsed figure in the line.
    started: Instant,
    /// When this task last produced a LINE. Written from TeeWriter::write on
    /// the tokio drain tasks, read from the ticker thread, so an atomic rather
    /// than a second mutex: a counter needs no lock ordering against
    /// TERMINAL_LOCK.
    last_line_ms: AtomicU64,
    /// When this task was last heartbeated, so spacing is measured from the
    /// previous beat rather than from the task start.
    last_beat_ms: AtomicU64,
}
```

Both millisecond fields are offsets from a single run-start `Instant`, so they
are monotonic and immune to wall-clock adjustment.

The ottofile key follows `jobs` exactly (`src/cfg/otto.rs:294`, comment at
`:279-293`): an `Option<u64>` on `OttoSpec`, so "unset" and "explicitly set"
stay distinguishable and the flag can win via `matches.value_source`
(`src/cli/parser.rs:964`,`:984`). Kebab on disk with an explicit
`#[serde(rename = "progress-interval")]`, matching `envs-command`
(`src/cfg/otto.rs:305`).

```yaml
otto:
  api: 1
  progress-interval: 10   # seconds of task silence before a heartbeat; 0 disables
```

`OttoSpec` is `#[serde(deny_unknown_fields)]` (`src/cfg/otto.rs:267`), so this
is a schema change and `tests/roundtrip.rs` must cover it.

### API Design

One global flag, declared in `Parser::global_args()`
(`src/cli/parser/help.rs:10-75`), the single source both parsing and rendered
help consume:

```
--progress-interval <SECONDS>  Seconds of task silence before otto reports the
                               task is still running; 0 disables
                               [env: OTTO_PROGRESS_INTERVAL=] [default: 10]
```

Precedence copied from `jobs`: flag > env > ottofile > default.

**One knob, not a tri-state.** `--progress=auto|always|never` is the animated
tools' surface (cargo, docker, bazel) and it exists because those tools must
decide whether a terminal can be drawn on. A heartbeat has no such question, so
an `auto` that always equals `always` would be theater. `0` is the off switch.

### Implementation Plan

#### Phase 0: Measure what heartbeat lines break in the existing suite
**Model:** opus
- Zero production code. Scratch branch, nothing merged.
- Patch a throwaway thread that prints `[probe] still running (0.0s)` to stderr
  every 1 second for the whole run, ungated.
- Run `otto ci`, and the five pty-based suites by name: `cancel_reaping_test`,
  `sigint_cancel_test`, `cli_surface_test`, `tasks_flag_test`, `tty_task_test`.
- `pty_stdout` (`tests/common/mod.rs:118-123`) strips a `^D` prefix, leading
  `\u{8}` bytes and all `\r`, but no other escape, and several suites assert with
  `contains` over whole-stderr text, so the failure set is not predictable by
  reading.
- **Success criteria:** a written list of every test that fails under a
  1-second ungated heartbeat, each with the assertion line that broke. An empty
  list is a valid and welcome result, and it is recorded in this doc either way.
- Kept, but it is not the load-bearing unknown. It measures how brittle the
  existing suite is to extra stderr lines. The assumptions correctness rests on
  are settled above by reading source: the colour predicate
  (`colored-3.1.1/src/control.rs:107`), `blocking_lock` being legal off-runtime,
  `report_status_line` being `async fn`, and the ticker lifetime.

**RESULT (2026-09-15, run on a scratch branch off `b51b509`, nothing merged).**
The probe was an ungated `std::thread` in `main()`, immediately after
`info!("Starting otto")`, printing `[probe] still running (0.0s)` to stderr.
Presence in the built binary was confirmed (`strings target/debug/otto`) and
firing was confirmed on a live run (30 lines on a 30-second silent task), so a
green suite is evidence rather than an untested probe.

*The doc's specified 1-second interval measures almost nothing, and that is the
first finding.* At 1s, `cargo test --workspace --all-features` was **green, 49
suites, 0 failures** - but `cli_surface_test` runs 14 tests in 0.05s and
`tasks_flag_test` 8 tests in 0.04s, so the probe never fired once inside them.
An empty failure set at 1s must not be read as "the suite tolerates extra stderr
lines". The measurement was redone with the probe firing immediately and every
100ms, so every otto invocation emits at least one line, and with
`--no-fail-fast` (the first aggressive run stopped at the first failing binary
and left 16 suites unrun, which would have been a second false empty).

**Failure set under the immediate/100ms probe: 2 tests out of 49 suites.**

| Test | Assertion that broke |
|---|---|
| `makefile_converter_test::test_strict_passes_a_makefile_that_converts_cleanly` | `tests/makefile_converter_test.rs:755`, `assert!(stderr.is_empty(), "{stderr}")` |
| `tasks_flag_test::tasks_defaults_to_yaml_on_a_real_tty` | `tests/tasks_flag_test.rs:298`, `tty output must be valid YAML: deserializing from YAML containing more than one document is not supported`. Under `script` the pty merges stderr into stdout, so a heartbeat line became a second YAML document ahead of `down:` |

**Both are artifacts of the probe's placement, and that is what makes them
useful.** The probe sits in `main()` and is ungated, so it fires for
invocations that execute no task at all: `otto makefile convert` and
`otto --tasks` both print and exit without ever reaching `execute_all`. The
shipped ticker is armed in the scheduler, so neither invocation would arm one.

The finding that changes Phase 3 is therefore a constraint, not a test-fix list:

- **The ticker must be armed no earlier than the run, and never for a
  non-executing invocation** (`--tasks`, `--help`, `--list-subtasks`,
  `makefile convert`, every other subcommand). These two tests are the canary
  for getting that wrong, and they already exist, so Phase 3 needs no new test
  for the property. If either goes red in Phase 3, the ticker is armed too early.
- `tasks_defaults_to_yaml_on_a_real_tty` also records the one place the
  stdout/stderr split the design relies on genuinely collapses: under a pty they
  are the same stream. That costs nothing here only because `--tasks` runs no
  task. A future feature that emitted heartbeats during a `--tasks`-style data
  dump would corrupt a parser, and this test would catch it.
- The remaining 47 suites are insensitive to extra stderr lines even at 100ms,
  which is the reassurance Phase 0 was asked for.

#### Phase 1: Flag, env var, ottofile key. No behaviour.
**Model:** sonnet
- `--progress-interval` in `Parser::global_args()` (`src/cli/parser/help.rs`),
  with `.env("OTTO_PROGRESS_INTERVAL")`.
- `progress_interval: Option<u64>` on `OttoSpec` (`src/cfg/otto.rs:268`) with
  `#[serde(rename = "progress-interval")]`, resolved with the `jobs`
  `value_source` pattern (`src/cli/parser.rs:964`,`:984`).
- Thread `RunPlan` -> `RuntimeConfig` -> scheduler. Nothing reads it yet.
- **Success criteria:**
  - `EXPECTED_GLOBAL_OPTIONS_HELP_TEMPLATE` (`src/cli/parser_tests_b.rs:891`) is
    updated and `cargo test --lib` is green.
  - An ottofile setting `progress-interval: 30` round-trips byte-identically in
    `tests/roundtrip.rs`, and a misspelled sibling key still fails loudly.
  - `otto --progress-interval 5 <task>` parses and `otto --help` lists the flag.

#### Phase 2: The ledger and the idle clock. Nothing printed.
**Model:** opus
- `TaskClock` map keyed by task name, created when a task's `TaskStreams` are
  created. It carries NO liveness: liveness is read from `LiveChildren` at tick
  time, so there is no mirror and therefore no set of update sites to get wrong.
- `last_line_ms` bumped at the top of `TeeWriter::write`
  (`src/executor/output.rs:148`), above the `suppress_terminal` branch.
- `tty:` tasks are excluded at tick time by name: they have no `TaskStreams`
  (`task_execution.rs:290-295`), so no clock, so they are never candidates.
- **Success criteria:**
  - A test asserts the live-child count observed at tick time REACHES 2 and
    never exceeds 2 across a `foreach ... jobs: 2` over 8 items. Reaching 2 is
    the half that a permanently-empty read would otherwise pass.
  - A test asserts a completed task is absent from the tick-time candidate set
    after its status line while a still-running sibling remains present, and
    that a run cancelled with SIGINT leaves an empty candidate set (the
    `abort_all` path, `scheduler.rs:516`).
  - A test asserts `last_line_ms` advances for a buffered foreach subtask
    (whose bytes never reach the terminal) and does not advance on a sibling's
    output.

#### Phase 3: The ticker and the line.
**Model:** opus
- OS thread (`std::thread`, not `spawn_blocking`) waking at
  `min(interval, 1s)` granularity, taking `terminal_lock()` itself and writing
  to stderr. It does NOT call `report_status_line`, which is `async fn`
  (`replay.rs:435`) and whose first act is `replay_ready_blocks(..).await`. The
  emit path is a new synchronous function alongside it, sharing the lock
  discipline and nothing else. It must be lock-free with respect to
  `TERMINAL_LOCK`, which is a non-reentrant `std::sync::Mutex`
  (`output.rs:36`): the ticker takes the lock once and calls nothing that takes
  it again.
- Label from `status_label` (`scheduler.rs:1369`) when stderr is a terminal, and
  the plain `[name]` form otherwise. `status_label` reaches
  `colorize_task_prefix` (`colors.rs:70`), gated on `SHOULD_COLORIZE`, which
  `colored` derives from `stdout().is_terminal()`. Applying a stdout decision to
  stderr is how colour reaches a redirected stderr today; the heartbeat does not
  inherit it.
- Elapsed from the existing
  `format_duration` (`src/cli/commands/format.rs:26`), not a new formatter. It
  yields `1m30s` above a minute and `45.0s` below one; the one-decimal form
  under 60s is a consequence of reuse, and reuse beats a second duration
  formatter that can drift.
- The thread runs for the whole run and is stopped by an explicit shutdown flag
  set after `execute_all` returns. The stop is only half of "no heartbeat can
  follow the final status line": a tick that has already chosen its lines can be
  waiting an arbitrarily long replay for the terminal lock while the task exits,
  deregisters and prints its own status line. So the write re-decides *under*
  that lock - shutdown flag, liveness and elapsed all re-read there - and emits
  nothing that is no longer true. **Corrected by the round-1 implementation
  audit,** which proved from source that the join alone made this an inference
  rather than a guarantee. It does NOT stop when no child is live: that state is reached before the
  first child spawns, between dependent tasks, and while a drain outlives its
  child (`scheduler.rs:271-276`). Stopping on first-empty would disable
  monitoring across exactly the silent dependency gap this is for.
- Suppressed when `tui_mode` is set, matching `replay.rs:457`.
- **Success criteria:**
  - A task that sleeps 25 seconds under `--progress-interval 5` emits at least 3
    lines matching `still running` naming that task; the first lands no earlier
    than 5s and no later than 7s after the task's last line, and consecutive
    lines are 5s to 7s apart.
  - The same run with both streams redirected to files produces **zero** `\r`
    bytes in either file, and **zero** `0x1b` bytes on stderr.
  - The same run with stdout on a pty and stderr redirected to a file produces
    **zero** `0x1b` bytes in that file, which is the split-redirection case
    otto's existing status lines fail today.
  - A task echoing once per second for 25 seconds under `--progress-interval 5`
    emits **zero** lines matching `still running`. This only guards anything
    alongside the positive criterion above: zero lines from a run that could
    never emit proves nothing.
  - A task that emits a `\r`-redrawn progress bar with no newline for 25 seconds
    IS heartbeated, since `read_until(b'\n')` never yields it a line. This is the
    motivating `philo slim-dump` shape and is the case the feature exists for.
  - `--progress-interval 0` on the 25-second silent task emits zero lines, and
    no line matching `still running` appears after the final status line on any
    of these runs.

#### Phase 4: Docs and coverage.
**Model:** sonnet
- README flag row; one annotated `progress-interval` line in the shipped example
  ottofile.
- **Success criteria:**
  - `otto ci` green, including the coverage floor measured on the runner rather
    than locally (`.otto.yml` documents that a local read runs about 4.5 points
    high).
  - `otto --help` and the README flag table both list `--progress-interval` with
    the same default. `docs-check` is a link checker and establishes nothing
    about completeness, so it is not the criterion here.

## Acceptance Criteria

Four of the five were run against `main` at `b51b509` before this doc was called
ready; the fifth says why it was not. The probe ottofile is three tasks: `quiet`
(echo, `sleep 30`, echo), `chatty` (echo once per second for 30 seconds), and
`boom` (echo, `exit 3`).

- [x] `otto --help | grep -c progress-interval` returns `1`.
  - **Observed on main:** `0`. Fails as expected until Phase 1.
  - **Observed at `7349e89` (Phase 1):** `1`. Criterion met. The rendered row is
    `--progress-interval <SECONDS>  Seconds of task silence before otto reports
    the task is still running; 0 disables [env: OTTO_PROGRESS_INTERVAL=]
    [default: 10]`. **Doc defect corrected:** the API Design block above
    originally wrote the two suffixes as `[default: 10] [env: ...]`. clap emits
    them in the opposite order, so the doc's rendering was never achievable and
    the golden snapshot reflects clap's order.
- [x] A 30-second silent task with both streams redirected to files yields zero
      `\x1b` bytes and zero `\r` bytes in both files.
  - **Observed on main:** `stdout: esc=0 cr=0`, `stderr: esc=0 cr=0`. Already
    true, so this is a regression guard, not a new property. The same task under
    `script` emits 18 escape bytes of colour, which is correct and unchanged.
  - **Observed at `02d7e76` (Phase 3), 30s silent task at `--progress-interval
    5`:** `stdout: bytes=74 esc=0 cr=0`, `stderr: bytes=149 esc=0 cr=0`, with 5
    `still running` lines in stderr. Criterion met, and now non-vacuous: the
    stderr it reports clean is stderr that actually carries heartbeat lines.
- [x] With stdout on a pty and stderr redirected to a file, no heartbeat line in
      that file carries an escape byte.
  - **Observed at `02d7e76` (Phase 3):** `script -qec "otto
    --progress-interval 5 quiet 2> pty-err.txt" /dev/null` put **0** escape
    bytes into `pty-err.txt` across 5 heartbeat lines (`bytes=149 esc=0 cr=0`),
    while the pty stdout carried 18 escape bytes of colour as it should.
    Criterion met: this is the split-redirection case otto's existing status
    lines still fail, and the heartbeat's own label gating is what avoids it.
  - **Observed on main:** the analogous existing line DOES carry them. `script
    -qec "otto boom 2> err.txt" /dev/null` put 6 escape bytes into `err.txt`:
    `^[[91m[^[[0m^[[92mboom^[[0m^[[91m]^[[0m failed`. This is otto's existing
    behaviour, not something this feature introduces, and it is why the
    heartbeat gates its own label on stderr rather than reusing `status_label`
    unconditionally. Fixing it for otto's status lines generally is out of scope
    here and named in Risks.
- [x] A task printing once per second for 30 seconds yields exactly zero lines
      matching `still running`.
  - **Observed on main:** `0` (31 stdout lines, none matching). Guards the
    idle-clock reset; breaking the reset must fail this.
  - **Observed at `02d7e76` (Phase 3):** `0` matching, 31 stdout lines, and
    stderr is `0 bytes`. Paired with the positive criterion below, which emits 5
    lines on the same binary, so this zero is a measured reset rather than a run
    that could not emit.
- [x] A 30-second silent task at the default interval yields at least 2 lines
      matching `still running` that name the task.
  - **Observed at the round-1 audit remediation, AT THE DEFAULT INTERVAL (no
    flag, no `OTTO_PROGRESS_INTERVAL`, no `otto:` block):** `2` lines, both
    naming `[quiet]`, at **11.0s** and **21.0s** of a 30-second run whose own
    `START` landed at 0.03s; stderr `bytes=60 esc=0 cr=0`. Criterion met on the
    interval it names. **Corrected by the round-1 implementation audit:** every
    figure previously recorded against this line was measured at
    `--progress-interval 5`, which is not the default the criterion claims.
  - **Observed on main:** `0`. Cannot pass before Phase 3, by construction.
  - **Observed at `02d7e76` (Phase 3), at `--progress-interval 5`** - kept as
    evidence for the spacing and `\r`-bar properties, not for the default: `5` lines,
    all naming `[quiet]`. First beat **5.99s** after the task's own first line,
    inside the 5-7s window; consecutive gaps **5.00s, 5.00s, 5.00s, 5.00s**.
    Elapsed renders through `format_duration` as `(6.0s)` ... `(26.0s)`.
  - **Also verified, the `\r`-bar case the feature exists for:** a task
    redrawing a bar with `printf '\r...'` and no newline for 30s emitted `5`
    `[crbar] still running` lines. Its own 60 `\r` bytes are on stdout; otto's
    stderr is `esc=0 cr=0`. This is the `philo slim-dump` shape.
  - **Also verified, `--progress-interval 0`:** `0` lines, stderr `0 bytes`. And
    on a merged-stream run the tail is `still running (26.0s)` -> `quiet: done`
    -> `finished successfully`, so no beat follows the final status line.
- [ ] `otto ci` is green on the runner.
  - **UNVERIFIED pending a push.** The branch has never left this machine
    (`git log origin/main..HEAD` is every commit of this feature), so no runner
    has ever run it and there is no runner figure to claim this on. Every
    reading below is LOCAL. **Caught by the round-1 implementation audit:** this
    box was previously ticked on a local read annotated "the runner's figure is
    the one this criterion is claimed on", which was a criterion bent to fit
    rather than a doc defect corrected. It stays unticked until the branch is
    pushed and the runner reports; the local readings are real evidence that the
    suite is green, just not the evidence this bullet asks for.
  - **Observed LOCALLY at the round-1 audit remediation, 2026-09-15:** `[ci] ✅
    All CI checks passed!`. Coverage `Lines: 94.6% (26260/27765)`, over the 87%
    floor. A local read, which `.otto.yml` documents as running about 4.5 points
    high against the runner's pinned cargo-llvm-cov.
  - **Observed LOCALLY at `efdddcb` (Phase 4, final), 2026-09-15:** `exit 0`,
    `[ci] ✅ All CI checks passed!`. Coverage `Lines: 94.6% (26173/27679)`, over
    the 87% floor, and up from the baseline's 94.5% rather than down. Re-run
    independently of the phase agents' own runs.
  - **Observed locally on main at `b51b509`, 2026-09-15:** `exit 0`, `[ci] ✅
    All CI checks passed!`. Coverage read `Lines: 94.5% (25375/26850)`, over the
    87% floor; that is a LOCAL read, which `.otto.yml` documents as running
    about 4.5 points high against the runner's pinned cargo-llvm-cov, so the
    runner figure is the one this criterion is claimed on. The green baseline is
    now measured rather than asserted by construction, and each phase re-runs
    the gate.

## Resolved Decisions

- **2026-09-15, heartbeat rather than an animated spinner.** The requirement is
  "the user can tell it isn't frozen", not "an animation exists". The deciding
  argument is NOT "no escape bytes": that claim was false and is corrected in
  Goals. It is that the drain reads `read_until(b'\n')` (`output.rs:250`), so a
  `\r`-redrawn progress bar never yields a line, never resets the clock, and is
  heartbeated. An `is_terminal`-gated animation shows nothing for that same case
  in CI, under `2>&1 | tee`, or in a captured log. That those are the cases where
  "is it hung?" gets asked most is the author's judgement, not measured.
  Recorded against Ian's literal wording ("a little ascii spinner")
  and his own fallback in the same thread ("or even just a timer counting").
- **2026-09-15, build this rather than PR #9.** Review panel round 1, Architect
  (Gemini) and Staff Engineer (Codex): verdict "build the heartbeat, revised; do
  not merge PR #9". The Architect seat dissented for PR #9 as the base on two
  grounds, both refuted in the panel's own synthesis: that this design would put
  30 lines into a log (it emits only for a task with no newline-terminated output
  for the interval), and that its buffered-subtask gap is fatal (the motivating
  `\r`-bar case does not reset the clock, so it is heartbeated). No unrefuted
  concrete flaw remains, so the dissent is recorded and not carried forward.
  Full findings: `/tmp/review-panel/rqUhvkxf/synthesis.md`.
- **2026-09-15, one definition of "still running".** A live child that has
  produced no line for N seconds. Liveness from `LiveChildren`, silence from the
  drain. Drafts of this doc used three different definitions in three places
  (liveness, observed lines, terminal visibility); the third is dropped
  entirely. Raised by both review seats as the hardest question in the design.
- **2026-09-15, stderr rather than stdout,** although otto's success lines go to
  stdout (`scheduler.rs:1683`). A heartbeat is ephemeral chatter, not a result:
  stdout stays the data channel that `--tasks` and downstream parsers rely on,
  and `2>/dev/null` is a familiar off switch that costs nothing to implement.
  The asymmetry with success lines is intentional and recorded here so it is not
  re-litigated as a bug.
- **2026-09-15, per-task lines rather than one aggregated line.** A run-wide
  idle clock is reset by any task's output, so a single chatty task masks a
  stalled sibling forever. That is the reported failure reintroduced. The output
  cost is bounded: a task emits only while silent, so N lines per interval means
  N genuinely stalled tasks.
- **2026-09-15, default 10 seconds, on by default.** Terraform's
  `defaultPeriodicUiTimer`. Ian floated "5 (3?)"; 3 seconds on a bursty task
  produces noticeably more lines for no extra information, and the key is
  configurable for anyone who disagrees. On by default because a diagnostic you
  must know to enable does not get enabled by the person who needs it.
- **2026-09-15, no backoff.** Bazel grows its interval 10s -> 30s -> 60s and has
  a standing bug where it keeps growing past the documented interval
  (bazelbuild/bazel#16119). Terraform has no backoff at all. Fixed interval, so
  that bug class cannot exist here.

## Alternatives Considered

### Alternative 1: Animated ANSI spinner on stderr, tty-gated
- **Description:** draw `⠋ 3 running: philo, catalog-api · 0:42` in place with
  `\r` plus `\x1b[K`, only when stderr is a terminal, erasing before every other
  write.
- **Pros:** closer to the literal words of the request. Zero added lines on an
  interactive terminal. Matches cargo, docker, gh.
- **Cons:** acquires obligations the heartbeat never has. A tty predicate that
  must be computed once and never re-derived (npm shipped that bug, commit
  `5b858c6`). An erase-before-every-write invariant every current and future
  terminal writer must honour or the screen corrupts. Width truncation, because
  a wrapped frame occupies two rows while the erase clears one. Cursor
  restoration on SIGINT and panic. A contaminated pty test matrix, since
  `pty_stdout` (`tests/common/mod.rs:119-123`) strips `\r` but not `\x1b[K`. And
  it shows nothing in CI, under `tee`, or in a captured log, which are the cases
  where "is it hung?" gets asked most.
- **Why not chosen:** it buys an animation and pays in a permanent invariant.
- **Revisit condition:** a user running otto interactively reports the heartbeat
  lines are too noisy in practice. The animation is then a strictly additive
  upgrade on the tty path, and the ledger and idle clock built here are exactly
  what it needs.

### Alternative 2: Global aggregated line instead of per-task
Covered in Resolved Decisions. Rejected because a run-wide idle clock lets one
chatty task mask a stalled sibling.

### Alternative 3: `/dev/tty` so the indicator survives redirection
- **Why not chosen:** contradicts an otto invariant. Non-`tty:` children get
  their own session precisely so they cannot reach the controlling terminal
  (`task_execution.rs:273-285`, documented at `:234-267`). otto reaching around a
  redirect it enforces on its own children is incoherent, and it would defeat
  `otto build 2>/dev/null`.

### Alternative 4: Route the heartbeat through `TeeWriter`
- **Why not chosen:** `TeeWriter::write` always writes the log file first
  (`src/executor/output.rs:150`). The per-task `stdout.log` / `stderr.log` hold
  the child's captured bytes and are replayed verbatim for buffered foreach
  blocks (`read_output`, `output.rs:281-307`). Injecting otto-authored lines into
  them corrupts a stored artifact, which is a worse version of the problem this
  design exists to avoid.

## Known Limits

- **A buffered foreach subtask that is newline-chatty into its buffer is not
  heartbeated,** because the clock reset sits above the `suppress_terminal`
  branch (`output.rs:148` vs `:155`). Its bytes reach the terminal only through
  ordered replay, so the terminal can look idle while the clock says otherwise.
  The gap is narrower than it sounds: it needs EVERY live task to be buffered and
  newline-chatty at once, and if the subtask blocking replay is the wedged one,
  it is idle and is heartbeated correctly. The alternative placement, keying the
  clock on terminal writes, would heartbeat every chattering buffered subtask
  continuously, which is worse.
- **No bounded latency.** See the replay-batch note in Architecture.
- **`tty:` tasks are invisible to this,** stated in Non-Goals.
- **Rollback.** `OttoSpec` is `deny_unknown_fields` (`src/cfg/otto.rs:267`), so a
  binary predating Phase 1 rejects an ottofile carrying `progress-interval`.
  Ship the binary before shared ottofiles adopt the key, and remove the key
  before rolling a binary back.

## Technical Considerations

### Dependencies

Zero new crates, direct or transitive. The heartbeat needs `std::time`,
`std::thread`, and `std::sync::atomic`. `indicatif` 0.18.3 and `console` 0.16.2
are already direct dependencies (`Cargo.toml:39`,`:49`) and stay unused by this
path.

### Performance

- One extra OS thread for the duration of a run, sleeping almost all of it.
- One relaxed atomic store per line of task output, on a path that already does
  a file write plus a terminal write plus a flush syscall per line
  (`output.rs:168-172`). Not measurable.
- The ticker takes `terminal_lock()` only when it has a line to emit.

### Security

None. No new input parsed, no new file written, no new process spawned. The
ottofile key rides the existing `deny_unknown_fields` schema, so a typo is a
loud load error rather than a silent default.

### Testing Strategy

- **Byte-level guard, the one that matters:** run a silent task with output
  captured, assert zero `\r` on both streams and zero `\x1b` on stderr, in both
  the both-streams-redirected and the stdout-pty/stderr-file cases. Not zero
  `\x1b` on stdout: otto legitimately colours stdout when stdout is a terminal.
  This is the regression test lefthook lacked (evilmartians/lefthook#1539: a
  100ms `\r` spinner with no tty check put 300+ junk lines into a 30-second CI
  job).
- **Negative case:** a chatty task emits zero heartbeats. Break the idle-clock
  reset and this test must fail.
- **Buffered-subtask case:** a chattering foreach subtask with
  `suppress_terminal` set emits zero heartbeats. This is the case a
  terminal-write-keyed clock would get wrong, so it gets its own test.
- **Liveness correctness:** `foreach ... jobs: 2` over 8 items reaches 2 live
  children and never exceeds 2. Catches a regression to the `ActiveTasks`
  admission ledger, and the lower bound catches a read that is simply always
  empty.
- **Cancellation:** SIGINT mid-run emits no heartbeat afterwards, covering
  `abort_all` (`scheduler.rs:516`), the path a mirror would have missed.
- **Golden test:** `EXPECTED_GLOBAL_OPTIONS_HELP_TEMPLATE`
  (`src/cli/parser_tests_b.rs:891`) is the only verbatim output snapshot in the
  repo and must be updated in Phase 1.

### Rollout Plan

Single repo. No cross-repo blast radius and no ship-order constraint. otto's CI
dogfoods the binary (`cargo install --path . --locked`, then `otto quick`), so
the first CI run after merge exercises the feature against otto's own build.

Default on at 10 seconds. Anyone who wants it gone sets `progress-interval: 0`
in their ottofile or passes `--progress-interval 0`.

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Heartbeat lines break existing `contains` assertions in the pty suites | Med | Med | Phase 0 measures it before any production code exists |
| A stale clock entry is read for a dead task | Low | Low | Liveness is read from `LiveChildren` itself, so a clock entry with no live child is never a candidate. No mirror exists to drift |
| otto colours a redirected stderr because `colored` reads stdout | High | Low | Pre-existing and measured (see Acceptance Criteria). The heartbeat gates its own label; correcting otto's status lines generally is a separate change and is not folded in here |
| A heartbeat lands between a buffered block's lines | Low | Med | Emission takes `terminal_lock()`, which `write_replay_blocks` holds for a whole block (`replay.rs:375`) |
| CI logs grow | Low | Low | One short line per stalled task per interval. A task that prints stays silent here |
| A `tty:` task looks unmonitored | Low | Low | Stated as a non-goal with its reason. otto cannot see those bytes at all |

## Open Questions

None. Every question raised in drafting or in review is recorded in Resolved
Decisions with its reasoning.

## References

- Slack thread, DM `D084N7HV19S`, 2026-09-15
- `hashicorp/terraform`, `internal/command/views/hook_ui.go`: `defaultPeriodicUiTimer = 10 * time.Second`, `Still creating... [00m10s elapsed]`
- `bazelbuild/bazel` issue 16119: unbounded progress-report backoff
- `evilmartians/lefthook` issue 1539: ungated `\r` spinner, 300+ lines per CI job
- `npm/cli` commit `5b858c6`: progress predicate computed twice and diverged
- `docs/design/2026-08-31-buffered-foreach-computed-envs-required-params.md`: the terminal-lock design this builds on
