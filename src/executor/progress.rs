//! Idle-triggered activity spinner.
//!
//! The problem it solves: a task that spends four minutes downloading prints
//! nothing while it does, and otto looks frozen. The constraint it must not
//! break, stated in the same conversation that asked for it: a spinner that
//! ends up in a log file is worse than no spinner at all.
//!
//! Two rules keep both true, and neither is a heuristic.
//!
//! **Frames go to stderr, and only when stderr is a terminal.** Not stdout --
//! stdout is otto's data channel, carrying task output, status lines and
//! replayed blocks, and a frame there would corrupt anything parsing it. Not
//! `/dev/tty` either: otto deliberately denies its children a controlling
//! terminal (`task_execution.rs`, `setsid`), and reaching around a redirect to
//! the real terminal would contradict that and defeat `otto build 2>/dev/null`.
//! `is_terminal()` on stderr is the test that makes every capture case safe at
//! once -- `2>&1 |`, `2>file`, `$(… 2>&1)`, and a nested otto, whose stderr is
//! a pipe because children are spawned with `Stdio::piped()`.
//!
//! **Frames appear only after a quiet period.** A spinner that runs from the
//! first moment spends its life being erased and redrawn around output that is
//! already telling the user what is happening. This one waits for
//! [`IDLE_BEFORE_SPIN`] of silence before drawing anything, which is both what
//! was asked for -- "if a process is running and there hasn't been a line
//! logged in the last few seconds" -- and what makes the interleaving question
//! almost moot: by construction there is nothing to interleave with.
//!
//! The erase discipline itself belongs to the process-wide terminal lock in
//! `output.rs` rather than to this module. Every site that writes to otto's own
//! stdout or stderr already takes that lock; making the lock hide the frame on
//! acquire and reconsider it on release means no call site has to remember to,
//! and a replayed foreach block -- which holds the lock for its whole length --
//! stays as contiguous as it was before.

use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a run must go without printing before a frame appears.
///
/// Three seconds rather than one: the point is to distinguish "working" from
/// "wedged", and anything fast enough to finish inside three seconds never
/// looked wedged. It also keeps the spinner away from chatty tasks entirely.
const IDLE_BEFORE_SPIN: Duration = Duration::from_secs(3);

/// How often a visible frame advances. 100ms is fast enough to read as motion
/// and slow enough that the escape traffic is irrelevant next to the task
/// output it is waiting for.
const TICK: Duration = Duration::from_millis(100);

/// Braille frames: one cell wide in every terminal that renders them, so the
/// erase is always a single `\r` plus clear-to-end-of-line.
const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// What the spinner knows. Behind one mutex because the ticker thread and
/// whichever task thread is writing both reach it.
struct State {
    /// Tasks currently in flight, in the order they started. Named rather than
    /// counted so the frame can say what is actually taking the time.
    running: Vec<String>,
    /// Index into [`FRAMES`].
    frame: usize,
    /// Whether a frame is currently on screen and therefore needs erasing
    /// before anything else is written.
    drawn: bool,
    /// When otto last wrote real output. The idle rule measures from here.
    last_output: Instant,
    /// When the run started, for the elapsed counter -- the other half of what
    /// was asked for ("or even just a timer counting").
    started: Instant,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

/// Set once, at startup, and read on every write. Kept separate from [`STATE`]
/// so the disabled case -- every CI run, every pipe -- costs one atomic load
/// and never touches the mutex.
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn enabled() -> bool {
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            running: Vec::new(),
            frame: 0,
            drawn: false,
            last_output: Instant::now(),
            started: Instant::now(),
        })
    })
}

/// Whether a spinner should run at all, as a pure function of the things that
/// decide it.
///
/// Pure, and taking its environment as a closure, so every row of the table
/// below is a unit test rather than a process-level experiment with
/// `set_var` -- the same shape `choose_format` uses in `cli/commands/tasks.rs`.
///
/// | condition | why it disables |
/// |---|---|
/// | stderr is not a terminal | the whole rule; covers pipes, files, nested otto, CI capture |
/// | `--no-progress` / `OTTO_NO_PROGRESS` | the explicit opt-out, for a tty that is being recorded |
/// | `CI` non-empty | a build log is not a terminal in the sense that matters, whatever the pty says |
/// | `TERM` unset or `dumb` | `is_terminal()` does not check this, and a dumb terminal cannot erase a line |
/// | `NO_COLOR` | otto already honours it for prefixes; animating while suppressing colour is incoherent |
/// | `--tui` | the TUI owns the alternate screen and suppresses every other terminal write |
pub(crate) fn should_enable<F>(stderr_is_terminal: bool, tui_mode: bool, no_progress_flag: bool, env: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    if !stderr_is_terminal || tui_mode || no_progress_flag {
        return false;
    }
    if env("OTTO_NO_PROGRESS").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if env("CI").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if env("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    match env("TERM") {
        None => false,
        Some(t) if t.is_empty() || t == "dumb" => false,
        Some(_) => true,
    }
}

/// Turn the spinner on for this run, and start the thread that advances it.
///
/// A plain OS thread rather than a tokio task: it takes the terminal lock, a
/// `std::sync::Mutex` that is documented never to be held across an `.await`.
/// The same reasoning already puts replay's locked work in `spawn_blocking`.
pub(crate) fn configure(on: bool) {
    ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
    if !on {
        return;
    }
    {
        let mut st = state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        st.last_output = Instant::now();
        st.started = Instant::now();
    }
    std::thread::Builder::new()
        .name("otto-progress".into())
        .spawn(|| {
            while enabled() {
                std::thread::sleep(TICK);
                let _terminal = crate::executor::output::terminal_lock_raw();
                draw_if_idle();
            }
        })
        .ok();
}

/// A task began. Called from the one place the scheduler already marks a start.
pub(crate) fn started(name: &str) {
    if !enabled() {
        return;
    }
    let mut st = state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    st.running.push(name.to_string());
}

/// A task ended, however it ended -- success, failure, or skipped. The frame
/// says what is still running, so every terminal transition has to arrive here
/// or a finished task keeps being advertised.
pub(crate) fn finished(name: &str) {
    if !enabled() {
        return;
    }
    let mut st = state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(i) = st.running.iter().position(|n| n == name) {
        st.running.remove(i);
    }
}

/// Erase a visible frame. Called by the terminal lock on acquire, so that
/// whatever is about to be written lands on a clean line.
pub(crate) fn hide() {
    if !enabled() {
        return;
    }
    let mut st = state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    erase(&mut st);
}

/// Note that real output just happened, and reconsider the frame.
///
/// Called by the terminal lock on release. Resetting the idle clock here is
/// what makes a chatty run show no spinner at all: the redraw below cannot fire
/// until the run has been quiet for [`IDLE_BEFORE_SPIN`] again.
pub(crate) fn after_write() {
    if !enabled() {
        return;
    }
    let mut st = state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    st.last_output = Instant::now();
    // Deliberately no redraw: the clock was just reset, so there is nothing to
    // draw, and calling it would only re-check what we already know.
    let _ = &st;
}

/// Draw a frame if the run has been quiet long enough and something is running.
/// Called by the ticker with the terminal lock already held.
pub(crate) fn draw_if_idle() {
    if !enabled() {
        return;
    }
    let mut st = state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if st.running.is_empty() || st.last_output.elapsed() < IDLE_BEFORE_SPIN {
        erase(&mut st);
        return;
    }
    let line = frame_line(
        FRAMES[st.frame % FRAMES.len()],
        &st.running,
        st.started.elapsed(),
        term_width(),
    );
    st.frame = st.frame.wrapping_add(1);
    let mut err = std::io::stderr();
    // No newline, ever: a frame that is never terminated cannot enter
    // scrollback. `\r` returns to column 0, the text overwrites, and the
    // clear-to-end-of-line removes whatever the previous, longer frame left.
    let _ = write!(err, "\r{line}\x1b[K");
    let _ = err.flush();
    st.drawn = true;
}

/// Remove any frame and stop. Called on every exit path -- normal return,
/// signal handler, and the panic hook -- because a frame left on screen is the
/// one piece of damage this feature can actually do.
pub(crate) fn clear() {
    if !enabled() {
        return;
    }
    ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Some(lock) = STATE.get() {
        let mut st = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        erase(&mut st);
    }
}

fn erase(st: &mut State) {
    if !st.drawn {
        return;
    }
    let mut err = std::io::stderr();
    let _ = write!(err, "\r\x1b[K");
    let _ = err.flush();
    st.drawn = false;
}

/// The frame's text, truncated to fit.
///
/// Pure so the truncation is testable, and truncated at all because a frame
/// that wraps occupies two rows while `\r` plus clear-to-end-of-line erases
/// one -- the leftover half-line being exactly the defaced output this feature
/// is not allowed to produce. One column is left spare so a frame ending in the
/// last cell cannot trigger the terminal's own wrap.
pub(crate) fn frame_line(spinner: &str, running: &[String], elapsed: Duration, width: usize) -> String {
    let secs = elapsed.as_secs();
    let clock = format!("{}:{:02}", secs / 60, secs % 60);
    let names = running.join(", ");
    let line = if running.len() > 1 {
        format!("{spinner} {} running: {names} · {clock}", running.len())
    } else {
        format!("{spinner} {names} · {clock}")
    };
    let budget = width.saturating_sub(1);
    if budget == 0 {
        return String::new();
    }
    if line.chars().count() <= budget {
        return line;
    }
    let mut out: String = line.chars().take(budget.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Terminal width, or a conservative default when it cannot be determined.
fn term_width() -> usize {
    console::Term::stderr().size().1 as usize
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
