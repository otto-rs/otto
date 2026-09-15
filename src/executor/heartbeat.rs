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
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

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

#[path = "heartbeat_tests.rs"]
mod tests;
