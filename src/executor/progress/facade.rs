//! The one otto-owned terminal writer.
//!
//! Every site that writes to otto's own stdout or stderr goes through here, so
//! a buffered-foreach block can be replayed as one contiguous run of lines
//! without a concurrently running task's output, status line, skip message,
//! failure message, or the run-cancelled message landing in the middle of it.
//!
//! This replaces the process-wide terminal-lock static that used to live in
//! `output.rs` (design doc
//! `docs/design/2026-09-16-live-progress-renderer.md`, Phase 2). A bare lock
//! was enough while otto's own writers were the only ones: it is not
//! enough once indicatif's ticker thread draws on the same terminal holding
//! only indicatif's locks and never otto's. The facade is the seam a renderer
//! can be installed behind in Phase 5 without touching a single caller.
//!
//! A `std::sync::Mutex` on purpose: it is only ever held across synchronous
//! writes, never across an `.await`, and replay runs inside `spawn_blocking`
//! where blocking is the point.

use std::{
    io::{self, Write},
    sync::{Mutex, MutexGuard, OnceLock, PoisonError},
};

/// Which of otto's own terminal streams a write is headed for.
///
/// Deliberately not `output::OutputType`, which names the stream a *task*
/// produced and is serialized into the run record. These are otto's own two
/// handles; the two enums answer different questions and coupling them would
/// put a display concern in a persisted type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Both streams, locked together, handed to a [`Facade::block`] closure.
///
/// Replay interleaves a subtask's stdout and stderr inside a single locked
/// block with per-stream flushes (`scheduler/replay.rs`), so a single
/// `&mut dyn Write` cannot express it and calling back into the facade would
/// break the no-reentry rule below.
pub struct BlockStreams<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

/// The process-wide output facade.
///
/// Three rules callers must keep, enforced by there being no other way in:
///
/// - nothing inside a [`Facade::block`] closure may re-enter the facade; the
///   ordering lock is not reentrant and would self-deadlock.
/// - nothing inside a `block` closure may touch a renderer row directly.
/// - all row mutation happens outside a block.
///
/// Only this module has to know those rules exist. That is the point: an
/// erase-before-write invariant behind a facade binds ONE module instead of
/// every current and future terminal writer.
pub struct Facade {
    /// Guards write ORDERING and nothing else, which is why poisoning is
    /// recovered rather than propagated: a panic while it was held leaves no
    /// invariant broken, and refusing to print for the rest of the run would be
    /// a worse failure than an interleaved line. indicatif does not do this -
    /// it unwraps - which is a second reason the lock stays otto's.
    order: Mutex<()>,
}

/// The one facade. A `OnceLock` rather than a threaded parameter, mirroring the
/// `static` lock it replaces, so no signature in the scheduler changes.
static FACADE: OnceLock<Facade> = OnceLock::new();

/// Reach the process-wide facade.
pub fn facade() -> &'static Facade {
    FACADE.get_or_init(Facade::new)
}

impl Facade {
    /// Not `pub`: callers reach the process-wide instance through [`facade`].
    /// Tests build their own to exercise the lock without racing the harness's
    /// captured output.
    fn new() -> Self {
        Self { order: Mutex::new(()) }
    }

    /// Take the ordering lock, recovering poisoning rather than propagating it.
    fn order(&self) -> MutexGuard<'_, ()> {
        self.order.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One write, ordered against every other facade write, then flushed.
    ///
    /// Flushing per write (rather than batching) is deliberate: task output is
    /// interleaved with the scheduler's own status lines, and buffering here
    /// would let a chatty task's lines arrive out of order relative to those.
    ///
    /// Best effort, never fatal, and never panicking: a run that ended because
    /// the terminal hung up has a stderr whose every write fails `EIO`, and the
    /// `print!`/`eprint!` macros this replaces panic on a failed write. There
    /// is nowhere left to report a failed terminal write to.
    pub fn write(&self, stream: Stream, bytes: &str) {
        let _order = self.order();
        match stream {
            Stream::Stdout => {
                let mut w = io::stdout().lock();
                let _ = w.write_all(bytes.as_bytes());
                let _ = w.flush();
            }
            Stream::Stderr => {
                let mut w = io::stderr().lock();
                let _ = w.write_all(bytes.as_bytes());
                let _ = w.flush();
            }
        }
    }

    /// A contiguous run of writes that no other writer may split.
    ///
    /// Buffered-foreach replay holds ONE of these for a whole block, never one
    /// per line. The closure's return value comes back so a caller can report
    /// what it managed to write without smuggling state out past the lock.
    pub fn block<R>(&self, f: impl FnOnce(&mut BlockStreams<'_>) -> R) -> R {
        let _order = self.order();
        let stdout = io::stdout();
        let stderr = io::stderr();
        let mut out = stdout.lock();
        let mut err = stderr.lock();
        let mut streams = BlockStreams {
            out: &mut out,
            err: &mut err,
        };
        f(&mut streams)
    }

    /// Put the terminal back the way otto found it, ahead of anything that
    /// writes on the way out.
    ///
    /// An explicit call, not a `Drop`: the facade lives in a `OnceLock`, whose
    /// statics never drop, and two exit paths (`app.rs`'s second-signal
    /// `process::exit` and `main.rs`'s fatal path) bypass unwinding entirely.
    ///
    /// Idempotent, because the exit paths overlap: a fatal error during a run
    /// that is also being signalled can reach two of them.
    ///
    /// Today this only flushes. There is no live region to erase until Phase 5
    /// of `docs/design/2026-09-16-live-progress-renderer.md` installs one; the
    /// call sites exist now so that phase adds a renderer and not a scavenger
    /// hunt for exit paths.
    pub fn teardown(&self) {
        let _order = self.order();
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
    }
}

#[path = "facade_tests.rs"]
mod tests;
