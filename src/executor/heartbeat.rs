//! Per-task idle timing for the `still running` heartbeat (design doc
//! `docs/design/2026-09-15-idle-task-heartbeat.md`).
//!
//! Timing, and nothing else. **Liveness is deliberately absent:** the only
//! thing in otto that means "a process exists right now" is the scheduler's
//! live-child registry, and a copy of it here would have to be updated at six
//! sites rather than the five that are obvious - `ActiveTasks::abort_all`
//! clears the whole registry on cancellation without any task body running, and
//! an aborted body never reaches the backstop that would have removed its own
//! entry. A mirror that missed that site would report tasks SIGKILLed by Ctrl+C
//! as still running for the rest of the run. A tick reads the registry itself
//! and uses this map only to ask how long each live task has been silent.

use std::{
    collections::HashMap,
    io::{self, Write},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use log::{debug, error};

use super::{
    colors::{stderr_takes_color, stream_task_label},
    output::terminal_lock,
};
use crate::cli::commands::format::format_duration;

/// The clock map, keyed by task name.
type ClockMap = HashMap<String, Arc<TaskClock>>;

/// One task's timing. NOT a liveness ledger; see the module comment.
pub struct TaskClock {
    /// The run-start instant both millisecond fields below are offsets from.
    /// A copy of `TaskClocks::origin` rather than a borrow, so stamping a line
    /// takes no lock: `TeeWriter::write` stamps once per line of task output,
    /// on the drain tasks.
    origin: Instant,
    /// For the elapsed figure in the line.
    started: Instant,
    /// When this task last produced a LINE. Written from `TeeWriter::write` on
    /// the tokio drain tasks, read from the ticker thread, so an atomic rather
    /// than a second mutex: a counter needs no lock ordering against
    /// `TERMINAL_LOCK`.
    last_line_ms: AtomicU64,
    /// When this task was last heartbeated, so spacing is measured from the
    /// previous beat rather than from the task start.
    last_beat_ms: AtomicU64,
}

impl TaskClock {
    /// A task that has just started has produced no line and taken no beat, so
    /// both offsets begin at this instant: a task is silent from the moment its
    /// clock starts, not from the run's start, which for a task that begins
    /// late in a run is the difference between one interval of silence and
    /// several.
    fn new(origin: Instant) -> Self {
        let started = Instant::now();
        let ms = offset_ms(origin, started);
        Self {
            origin,
            started,
            last_line_ms: AtomicU64::new(ms),
            last_beat_ms: AtomicU64::new(ms),
        }
    }

    /// This task just produced a line.
    ///
    /// `Relaxed` on purpose: the only reader compares this against a later
    /// reading of the same atomic, so there is no second location whose
    /// ordering against it matters, and this runs once per line of output.
    pub fn note_line(&self) {
        self.last_line_ms.store(self.now_ms(), Ordering::Relaxed);
    }

    /// This task was just heartbeated.
    pub fn note_beat(&self) {
        self.last_beat_ms.store(self.now_ms(), Ordering::Relaxed);
    }

    /// Offset, in milliseconds from the run's start, of this task's last line.
    pub fn last_line_ms(&self) -> u64 {
        self.last_line_ms.load(Ordering::Relaxed)
    }

    /// Offset, in milliseconds from the run's start, of this task's last beat.
    pub fn last_beat_ms(&self) -> u64 {
        self.last_beat_ms.load(Ordering::Relaxed)
    }

    /// How long this task has been silent.
    pub fn idle_ms(&self) -> u64 {
        self.now_ms().saturating_sub(self.last_line_ms())
    }

    /// How long since this task was last heartbeated.
    pub fn since_beat_ms(&self) -> u64 {
        self.now_ms().saturating_sub(self.last_beat_ms())
    }

    /// How long this task has been running.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn now_ms(&self) -> u64 {
        offset_ms(self.origin, Instant::now())
    }
}

/// Milliseconds from `origin` to `at`, monotonic and so immune to a wall-clock
/// adjustment. Saturating rather than panicking on an `at` that precedes
/// `origin`: no caller can produce one, and a heartbeat is the wrong place to
/// abort a run from.
fn offset_ms(origin: Instant, at: Instant) -> u64 {
    at.saturating_duration_since(origin).as_millis() as u64
}

/// Every non-`tty:` task's clock for one run, keyed by task name, with the
/// single run-start `Instant` all their offsets are measured from.
///
/// An entry may legitimately outlive its child: a clock whose task is not live
/// is never read, so a stale entry is inert rather than a bug.
pub struct TaskClocks {
    origin: Instant,
    clocks: Mutex<ClockMap>,
}

impl Default for TaskClocks {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            clocks: Mutex::new(HashMap::new()),
        }
    }
}

impl TaskClocks {
    /// Start `task`'s clock and hand back the handle its drains stamp.
    ///
    /// One clock per task, shared by both of its streams: either stream
    /// producing a line means the task is not silent. Called once, as the
    /// task's `TaskStreams` are created, so a task that never got as far as
    /// spawning a child never gets a clock either.
    pub fn start(&self, task: &str) -> Arc<TaskClock> {
        let clock = Arc::new(TaskClock::new(self.origin));
        self.map().insert(task.to_string(), clock.clone());
        clock
    }

    /// This task's clock, if it has one.
    pub fn get(&self, task: &str) -> Option<Arc<TaskClock>> {
        self.map().get(task).cloned()
    }

    /// The clocks a tick would consider, given who has a live child right now.
    ///
    /// An intersection, and the direction is the whole point: `live` is the
    /// authority on what exists and this map only says how long each has been
    /// silent. A clock whose task is not live is not read; a live task with no
    /// clock is not a candidate, which is how a `tty: true` task is excluded by
    /// name - it skips `TaskStreams` entirely, so it never had one. Ordered by
    /// name so a tick that emits several lines emits them in a stable order.
    pub fn candidates(&self, live: &[String]) -> Vec<(String, Arc<TaskClock>)> {
        let clocks = self.map();
        let mut candidates: Vec<(String, Arc<TaskClock>)> = live
            .iter()
            .filter_map(|task| clocks.get(task).map(|clock| (task.clone(), clock.clone())))
            .collect();
        candidates.sort_by(|(a, _), (b, _)| a.cmp(b));
        candidates
    }

    /// Take the map's lock, recovering from poisoning rather than propagating
    /// it - the same reasoning as `terminal_lock` (`output.rs`): a panic while
    /// it was held leaves no invariant broken here, and refusing to time
    /// anything for the rest of the run would be a worse failure than a clock
    /// read taken beside a task that panicked.
    fn map(&self) -> MutexGuard<'_, ClockMap> {
        self.clocks.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The longest a tick sleeps, so a 10-minute interval still notices a stop
/// within a second. The ticker wakes at `min(interval, MAX_WAKE)`, which is
/// also what bounds how late the first beat of a silent stretch can be.
const MAX_WAKE: Duration = Duration::from_secs(1);

/// A running ticker: the OS thread, plus what stops it.
///
/// An OS thread rather than a tokio task, and `std::thread` rather than
/// `spawn_blocking`, because the tick reads the live-child registry with
/// `tokio::sync::Mutex::blocking_lock`, which panics inside a runtime context.
/// It is also the reason replay does its locked terminal work off the async
/// path: a `std::sync` mutex must not be held across an `.await`.
pub struct Heartbeat {
    shutdown: Arc<Shutdown>,
    thread: Option<thread::JoinHandle<()>>,
}

/// The stop flag and the condvar the ticker sleeps on.
///
/// A condvar rather than a flag polled by a short sleep: the thread has to wake
/// on the interval rather than ten times a second, and stopping has to be
/// prompt - a plain `sleep(granularity)` loop would hold otto's exit for up to
/// a second after the run's final status line.
#[derive(Default)]
struct Shutdown {
    stopped: Mutex<bool>,
    wake: Condvar,
}

impl Shutdown {
    /// Sleep up to `granularity`. `true` means stop, and the lock is released
    /// before returning either way: a tick must not emit while holding it.
    fn sleep(&self, granularity: Duration) -> bool {
        let stopped = self.stopped.lock().unwrap_or_else(PoisonError::into_inner);
        if *stopped {
            return true;
        }
        let (stopped, _) = self
            .wake
            .wait_timeout(stopped, granularity)
            .unwrap_or_else(PoisonError::into_inner);
        *stopped
    }

    fn stop(&self) {
        *self.stopped.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.wake.notify_all();
    }

    /// Whether the run has asked the ticker to stop. Read under the terminal
    /// lock by a tick that has already decided to write, so a stop that landed
    /// while the tick waited for that lock still suppresses the line.
    fn is_stopped(&self) -> bool {
        *self.stopped.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Heartbeat {
    /// A handle that owns no thread, for the runs that emit nothing:
    /// `--progress-interval 0`, TUI mode, or a thread otto could not spawn.
    fn disabled() -> Self {
        Self {
            shutdown: Arc::new(Shutdown::default()),
            thread: None,
        }
    }

    /// Stop the ticker and wait for its thread.
    ///
    /// Called once `execute_all` has returned. The join bounds the ticker's
    /// lifetime; what makes "no heartbeat follows the final status line" a
    /// guarantee rather than an inference is `emit_due` re-deciding under the
    /// terminal lock, since a tick that selected its lines before the stop can
    /// still be waiting for that lock when this is called.
    /// Idempotent, which is what makes the `Drop` backstop free.
    pub fn stop(&mut self) {
        self.shutdown.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Heartbeat {
    /// Backstop for the paths that never reach an explicit `stop()` - an early
    /// return or an unwind - so a run can never leave a ticker beating.
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start the ticker for one run.
///
/// `live` is the tick's liveness read, supplied by the caller because the
/// registry it reads is a tokio mutex the caller owns; it is called from the
/// ticker thread, outside any runtime, which is what makes a `blocking_lock`
/// inside it legal. An interval of zero emits nothing and spawns nothing.
pub fn spawn<L>(interval: Duration, no_prefix: bool, clocks: Arc<TaskClocks>, live: L) -> Heartbeat
where
    L: Fn() -> Vec<String> + Send + 'static,
{
    if interval.is_zero() {
        debug!("heartbeat::spawn: disabled (--progress-interval 0)");
        return Heartbeat::disabled();
    }

    // Read once, here, and never re-derived: npm shipped a progress predicate
    // evaluated twice whose two readings diverged (commit 5b858c6), and nothing
    // about a run can change whether stderr is a terminal or what
    // `CLICOLOR_FORCE` said at startup. `stderr_takes_color` caches that read
    // for the whole binary, so the heartbeat and the status lines cannot
    // disagree about it.
    let stderr_color = stderr_takes_color();
    let granularity = wake_granularity(interval);
    let interval_ms = saturating_ms(interval);
    debug!(
        "heartbeat::spawn: interval={interval_ms}ms granularity={}ms stderr_color={stderr_color}",
        granularity.as_millis()
    );

    let shutdown = Arc::new(Shutdown::default());
    let signal = shutdown.clone();
    let stop = shutdown.clone();
    let thread = thread::Builder::new()
        .name("otto-heartbeat".to_string())
        .spawn(move || {
            while !signal.sleep(granularity) {
                tick(&clocks, &live, interval_ms, no_prefix, stderr_color, &stop);
            }
        });

    match thread {
        Ok(thread) => Heartbeat {
            shutdown,
            thread: Some(thread),
        },
        // The run is worth more than the progress notice, so a thread otto
        // could not start is reported and then done without.
        Err(e) => {
            error!("could not start the progress heartbeat thread: {e}");
            Heartbeat::disabled()
        }
    }
}

/// How long a tick sleeps between wakes: the interval, or [`MAX_WAKE`] if the
/// interval is longer.
///
/// Waking faster than the interval is what keeps the first beat of a silent
/// stretch close to the interval rather than up to a whole interval late, since
/// a task can fall silent at any point between two wakes.
fn wake_granularity(interval: Duration) -> Duration {
    interval.min(MAX_WAKE)
}

/// The interval in whole milliseconds, saturating rather than truncating.
///
/// `Duration::as_millis` is a `u128` and `as u64` keeps only its low 64 bits,
/// which lands `--progress-interval 2305843009213693952` on exactly zero. Zero
/// makes every live task due on every wake, so truncating turned the largest
/// value a user can reach for - "effectively never" - into the loudest setting
/// otto has.
fn saturating_ms(interval: Duration) -> u64 {
    u64::try_from(interval.as_millis()).unwrap_or(u64::MAX)
}

/// One wake: emit a line for every live task that is due one.
///
/// This reads liveness twice, and the second read is the point. The first is a
/// cheap filter outside the terminal lock, so a wake with nothing to say never
/// contends for it. The write itself then re-decides under the lock, because a
/// tick can wait an arbitrarily long replay there, and in that window the task
/// it selected can exit, deregister and have its own `finished successfully`
/// printed - leaving the beat to land after the line that says the task is
/// done. Deciding and writing have to be one critical section.
fn tick<L>(clocks: &TaskClocks, live: &L, interval_ms: u64, no_prefix: bool, stderr_color: bool, shutdown: &Shutdown)
where
    L: Fn() -> Vec<String>,
{
    if due_beats(clocks, &live(), interval_ms).is_empty() {
        return;
    }
    emit_due(clocks, live, interval_ms, no_prefix, stderr_color, shutdown);
}

/// The live tasks a tick should print a line for: silent for at least the
/// interval, and not already heartbeated within it.
///
/// Both halves matter. Silence alone would print a task that had been quiet for
/// a minute on every wake; the beat stamp alone would print a chattering task.
fn due_beats(clocks: &TaskClocks, live: &[String], interval_ms: u64) -> Vec<(String, Arc<TaskClock>)> {
    clocks
        .candidates(live)
        .into_iter()
        .filter(|(_, clock)| clock.idle_ms() >= interval_ms && clock.since_beat_ms() >= interval_ms)
        .collect()
}

/// One heartbeat line, terminated by an ordinary newline.
///
/// No carriage return, no cursor control, no erase: the line has to survive
/// being captured to a log verbatim, which is the constraint that chose a line
/// over an animated spinner. The label is uncoloured unless `stderr_color` says
/// stderr takes colour, rather than inheriting `task_label`'s stdout-derived
/// decision. That predicate is `colors::stderr_takes_color`, so a redirected
/// stderr IS coloured under `CLICOLOR_FORCE`: an explicit override outranks the
/// log-safety default, and this is the one case where a beat carries escapes
/// into a captured log.
/// Elapsed comes from otto's own `format_duration`, so `45.0s` under a minute
/// and `2m14s` over one.
fn beat_line(task: &str, elapsed: Duration, no_prefix: bool, stderr_color: bool) -> String {
    let label = stream_task_label(task, no_prefix, stderr_color);
    format!("{label} still running ({})\n", format_duration(elapsed.as_secs_f64()))
}

/// Re-decide and write a tick's lines to stderr under the process-wide
/// terminal lock.
///
/// Synchronous on purpose, and deliberately not `report_status_line`
/// (`scheduler/replay.rs`), which is an `async fn` whose first act is an
/// `.await`. The lock is taken exactly once for the whole tick, so its lines
/// stay contiguous, and nothing called under it takes it again: `TERMINAL_LOCK`
/// is a non-reentrant `std::sync::Mutex`. Taking the registry's tokio mutex
/// under it is safe in this one direction only, and it holds: no
/// `terminal_lock` caller takes the registry, and no registry holder takes
/// `TERMINAL_LOCK`.
///
/// Nothing is carried in from the pre-check - not the shutdown state, not the
/// candidate list, not the elapsed figure - because all three can go stale
/// while this waits for the lock.
fn emit_due<L>(
    clocks: &TaskClocks,
    live: &L,
    interval_ms: u64,
    no_prefix: bool,
    stderr_color: bool,
    shutdown: &Shutdown,
) where
    L: Fn() -> Vec<String>,
{
    let _terminal = terminal_lock();
    if shutdown.is_stopped() {
        return;
    }
    let due = due_beats(clocks, &live(), interval_ms);
    if due.is_empty() {
        return;
    }
    let mut err = io::stderr().lock();
    for (task, clock) in &due {
        let line = beat_line(task, clock.elapsed(), no_prefix, stderr_color);
        let _ = err.write_all(line.as_bytes());
    }
    let _ = err.flush();
    // Stamped after the write, not before: spacing is meant to be measured from
    // the line the user saw.
    for (_, clock) in &due {
        clock.note_beat();
    }
}

#[path = "heartbeat_tests.rs"]
mod tests;
