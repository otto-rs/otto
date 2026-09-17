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
    cell::RefCell,
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::{
        Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use log::debug;

use super::{
    mode::ProgressMode,
    ownership::{Surrendered, TerminalHandoff},
    region::{REFRESH_INTERVAL, Region},
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

impl Stream {
    /// This stream's slot in the facade's per-stream arrays.
    fn index(self) -> usize {
        match self {
            Self::Stdout => 0,
            Self::Stderr => 1,
        }
    }

    /// The other one. Line state propagates across this edge, and only when
    /// the two handles turn out to be the same destination.
    fn other(self) -> Self {
        match self {
            Self::Stdout => Self::Stderr,
            Self::Stderr => Self::Stdout,
        }
    }
}

/// The stream the live region draws on, named once rather than spelled
/// `Stream::Stderr` at each of the five sites that mean "the terminal the rows
/// are on" rather than "the stream this write is going to".
const REGION_STREAM: Stream = Stream::Stderr;

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
    /// `Some` while a `tty:` task owns the terminal, holding everything otto
    /// wrote in that window. Always taken AFTER `order`, at every site, so the
    /// handoff cannot land in the middle of a block.
    terminal: Mutex<Option<Surrendered>>,
    /// `Some` on the live path, from [`install_region`] until teardown. Always
    /// taken AFTER `terminal`, at every site: one lock order, stated once.
    region: Mutex<Option<Region>>,
    /// Whether the last byte the facade wrote to each stream was a newline,
    /// indexed by [`Stream::index`].
    ///
    /// This is the unterminated-final-chunk fix (design doc, "Unterminated
    /// final chunk"), and it is a pre-existing defect on `main`, not something
    /// the region introduced: `read_until(b'\n')` yields a final chunk with no
    /// newline at EOF, so `printf DONE` in a task put otto's own completion
    /// line on the end of the task's last line.
    ///
    /// Per stream, not one flag for both. One flag was right only when the two
    /// handles land on the same destination, and wrong in both directions
    /// otherwise: an unterminated chunk on stderr put a leading newline into a
    /// captured stdout that never needed one, and an unterminated chunk on a
    /// redirected stdout was answered by a newline sent to a stderr that was
    /// somewhere else entirely. [`Facade::one_destination`] is what re-couples
    /// them when they really are one terminal.
    at_line_start: [AtomicBool; 2],
    /// Whether otto's stdout and stderr resolve to the SAME open destination -
    /// one terminal, or one file both were redirected to.
    ///
    /// Decided once, from `fstat`, because it is a property of how otto was
    /// invoked and cannot change mid-run. When it holds, the two streams share
    /// one cursor and so share one line state; when it does not, a newline
    /// written to one says nothing about where the other is.
    one_destination: bool,
}

/// The one facade. A `OnceLock` rather than a threaded parameter, mirroring the
/// `static` lock it replaces, so no signature in the scheduler changes.
static FACADE: OnceLock<Facade> = OnceLock::new();

/// Reach the process-wide facade.
pub fn facade() -> &'static Facade {
    FACADE.get_or_init(Facade::new)
}

/// Arm the live region on the process facade and start the thread that keeps
/// its durations moving.
///
/// A free function rather than a method because of the thread: it refreshes
/// [`facade()`], the process instance, and a method on `&self` would let a test
/// holding a private `Facade` start a thread pointed at the shared one.
///
/// The thread is not joined and nothing waits for it. Its stop condition is the
/// region's absence, which teardown creates, so a run that exits leaves it at
/// most one [`REFRESH_INTERVAL`] asleep and a `std::thread` does not hold the
/// process open past `main`.
pub fn install_region(mode: ProgressMode, task_names: &[String]) {
    if !facade().set_region(mode, task_names) {
        return;
    }
    let spawned = std::thread::Builder::new().name("otto-progress".to_string()).spawn(|| {
        while facade().refresh() {
            std::thread::sleep(REFRESH_INTERVAL);
        }
        debug!("install_region: refresh thread stopping; the region is torn down");
    });
    if let Err(e) = spawned {
        // The rows would then sit at whatever they last said. Loud, and not
        // fatal: a run that cannot spawn a thread has bigger problems than a
        // stale duration, and the region still redraws on every otto write.
        log::warn!("install_region: could not start the progress refresh thread: {e}");
    }
}

impl Facade {
    /// Not `pub`: callers reach the process-wide instance through [`facade`].
    /// Tests build their own to exercise the lock and the ownership handoff
    /// without racing the harness's captured output.
    pub(super) fn new() -> Self {
        Self::with_one_destination(streams_share_destination())
    }

    /// [`Facade::new`] with the stdout/stderr sharing answer supplied rather
    /// than measured, so a test can assert both configurations without being
    /// at the mercy of how the harness wired its own handles.
    pub(super) fn with_one_destination(one_destination: bool) -> Self {
        Self {
            order: Mutex::new(()),
            terminal: Mutex::new(None),
            region: Mutex::new(None),
            at_line_start: [AtomicBool::new(true), AtomicBool::new(true)],
            one_destination,
        }
    }

    /// Take the ordering lock, recovering poisoning rather than propagating it.
    fn order(&self) -> MutexGuard<'_, ()> {
        self.order.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Take the ownership lock, recovering poisoning for the same reason
    /// [`Facade::order`] does.
    fn terminal(&self) -> MutexGuard<'_, Option<Surrendered>> {
        self.terminal.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Take the region lock, recovering poisoning for the same reason
    /// [`Facade::order`] does. A poisoned region lock means a panic happened
    /// with rows half-updated, which costs at worst one wrong row for one
    /// refresh interval; refusing to draw for the rest of the run would be the
    /// worse failure.
    fn region(&self) -> MutexGuard<'_, Option<Region>> {
        self.region.lock().unwrap_or_else(PoisonError::into_inner)
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
        self.deliver(stream, bytes);
    }

    /// One write that must START at column zero.
    ///
    /// Every line otto authors about a task goes through here - completion,
    /// failure, skip, the run-cancelled notice, a prune warning - because a
    /// task whose last chunk had no trailing newline leaves the cursor
    /// mid-line, and otto's own line then reads as a continuation of the
    /// task's: `[nonl] DONE[nonl] finished successfully`, measured on `main` at
    /// `1783f1c`. Replay already did this for a log whose last line was
    /// unterminated (`scheduler/replay.rs`); the live path never had.
    ///
    /// The guard is a newline, not a carriage return: the child's bytes are
    /// already on the screen and are not otto's to erase.
    pub fn write_line(&self, stream: Stream, bytes: &str) {
        let _order = self.order();
        if self.line_start(stream) {
            self.deliver(stream, bytes);
        } else {
            self.deliver(stream, &format!("\n{bytes}"));
        }
    }

    /// Send `bytes` where they are going, whatever otto is doing with the
    /// terminal right now. Caller holds `order`.
    ///
    /// Three destinations and one of them is not the terminal: held, while a
    /// `tty:` task owns it; through a region suspension, while rows are drawn;
    /// straight out, otherwise.
    fn deliver(&self, stream: Stream, bytes: &str) {
        let mut terminal = self.terminal();
        if let Some(held) = terminal.as_mut() {
            held.push(stream, bytes.as_bytes());
            self.note_written(stream, bytes.as_bytes());
            return;
        }
        drop(terminal);
        let mut region = self.region();
        match region.as_mut() {
            Some(region) => region.suspend(|| {
                raw_write(stream, bytes);
                self.note_written(stream, bytes.as_bytes());
                // Before the redraw `suspend` is about to do, which is the
                // design doc's "the facade emits a `\n` before ... a row draw
                // when the last byte it wrote was not one". Without it the
                // first row overwrites the child's unterminated last line, and
                // the next erase then deletes child output. The region's own
                // stream, not `stream`: this is about where the ROWS are about
                // to land, and a write to a redirected stdout moved no cursor
                // on the terminal they land on.
                self.ensure_line_start(REGION_STREAM);
            }),
            None => {
                raw_write(stream, bytes);
                self.note_written(stream, bytes.as_bytes());
            }
        }
    }

    /// Whether `stream`'s cursor is at column zero.
    fn line_start(&self, stream: Stream) -> bool {
        self.at_line_start[stream.index()].load(Ordering::Relaxed)
    }

    /// Record where `stream`'s cursor now is, and the other stream's too when
    /// the two are one destination.
    fn set_line_start(&self, stream: Stream, value: bool) {
        self.at_line_start[stream.index()].store(value, Ordering::Relaxed);
        if self.one_destination {
            self.at_line_start[stream.other().index()].store(value, Ordering::Relaxed);
        }
    }

    /// Record whether `stream`'s cursor is now at column zero.
    fn note_written(&self, stream: Stream, bytes: &[u8]) {
        if let Some(last) = bytes.last() {
            self.set_line_start(stream, *last == b'\n');
        }
    }

    /// Put `stream`'s cursor at column zero if it is not there already. Caller
    /// holds `order`, and the region (if any) is already cleared.
    fn ensure_line_start(&self, stream: Stream) {
        if !self.line_start(stream) {
            raw_write(stream, "\n");
            self.set_line_start(stream, true);
        }
    }

    /// A contiguous run of writes that no other writer may split.
    ///
    /// Buffered-foreach replay holds ONE of these for a whole block, never one
    /// per line. The closure's return value comes back so a caller can report
    /// what it managed to write without smuggling state out past the lock.
    pub fn block<R>(&self, f: impl FnOnce(&mut BlockStreams<'_>) -> R) -> R {
        let _order = self.order();
        let mut terminal = self.terminal();
        if let Some(held) = terminal.as_mut() {
            // A surrendered block is held exactly like a surrendered write, one
            // write at a time, so a replay that lands in this window stays
            // bounded instead of accumulating whole in a `Vec` first. The
            // `RefCell` is what lets both writers reach one sink; the closure is
            // single-threaded, so the two borrows never overlap.
            let sink = RefCell::new(held);
            let mut out = HeldWriter {
                stream: Stream::Stdout,
                sink: &sink,
            };
            let mut err = HeldWriter {
                stream: Stream::Stderr,
                sink: &sink,
            };
            let mut streams = BlockStreams {
                out: &mut out,
                err: &mut err,
            };
            return f(&mut streams);
        }
        drop(terminal);
        let mut region = self.region();
        match region.as_mut() {
            // ONE suspension for the whole block, never one per line: that is
            // what keeps a replayed group contiguous and what keeps a chatty
            // block from clearing and redrawing the region per line.
            Some(region) => region.suspend(|| self.block_on_terminal(f)),
            None => self.block_on_terminal(f),
        }
    }

    /// [`Facade::block`]'s terminal leg: the two locked handles, wrapped so the
    /// newline tracking sees what a block wrote. Caller holds `order`, and the
    /// region (if any) is suspended.
    fn block_on_terminal<R>(&self, f: impl FnOnce(&mut BlockStreams<'_>) -> R) -> R {
        let stdout = io::stdout();
        let stderr = io::stderr();
        let mut out = TrackedWriter {
            inner: &mut stdout.lock(),
            facade: self,
            stream: Stream::Stdout,
        };
        let mut err = TrackedWriter {
            inner: &mut stderr.lock(),
            facade: self,
            stream: Stream::Stderr,
        };
        let mut streams = BlockStreams {
            out: &mut out,
            err: &mut err,
        };
        let result = f(&mut streams);
        self.ensure_line_start(REGION_STREAM);
        result
    }

    /// Install the live region, once, and say whether it took.
    ///
    /// A no-op in [`ProgressMode::Quiet`], which is the whole tty gate: a
    /// captured run gets no region and so writes zero renderer bytes, by there
    /// being no renderer rather than by a renderer choosing to stay quiet.
    ///
    /// `task_names` is the run set, for the width of the name column. Read once
    /// here rather than recomputed per row, so the column does not jump as
    /// tasks come and go.
    pub(super) fn set_region(&self, mode: ProgressMode, task_names: &[String]) -> bool {
        if mode == ProgressMode::Quiet {
            return false;
        }
        let _order = self.order();
        let mut region = self.region();
        if region.is_some() {
            return false;
        }
        debug!("set_region: live region armed over {} task(s)", task_names.len());
        *region = Some(Region::new(task_names));
        true
    }

    /// A task started running: give it a row.
    ///
    /// Takes the task's own [`TaskClock`](crate::executor::clocks::TaskClock),
    /// not just its name. The design doc's API sketch passes a name alone, but
    /// the row has to read the same clock the completion line does or the two
    /// drift, which is the defect this phase exists to close; the clock is only
    /// in hand at the call site, so it comes in as an argument rather than
    /// being looked up in a second map.
    ///
    /// Outside any [`Facade::block`], per the facade's third rule.
    pub fn task_started(&self, name: &str, clock: std::sync::Arc<crate::executor::clocks::TaskClock>) {
        let _order = self.order();
        if self.terminal().is_some() {
            return;
        }
        if let Some(region) = self.region().as_mut() {
            self.ensure_line_start(REGION_STREAM);
            region.started(name, clock);
        }
    }

    /// A task reported: take its row off the screen.
    pub fn task_finished(&self, name: &str) {
        let _order = self.order();
        if self.terminal().is_some() {
            return;
        }
        if let Some(region) = self.region().as_mut() {
            self.ensure_line_start(REGION_STREAM);
            region.finished(name);
        }
    }

    /// Recompute every row from its clock, and say whether the region is still
    /// installed.
    ///
    /// The return value is the refresh thread's stop condition: teardown takes
    /// the region, so the absence of one IS "stop refreshing". No second flag to
    /// keep in step with the first.
    pub fn refresh(&self) -> bool {
        let _order = self.order();
        if self.terminal().is_some() {
            // Surrendered: the region is erased and the `tty:` child owns every
            // column. Still installed, so the thread keeps going.
            return self.region().is_some();
        }
        match self.region().as_mut() {
            Some(region) => {
                self.ensure_line_start(REGION_STREAM);
                region.refresh();
                true
            }
            None => false,
        }
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
    /// The region goes first, and it is ERASED rather than hidden: on a
    /// terminal draw target `disconnect` is a no-op, so
    /// `set_draw_target(hidden())` would leave the rows already on the screen
    /// exactly where they are and only stop future draws. Erasing before the
    /// held output is flushed is the same ordering every other site here keeps:
    /// otto's own words never land in rows that are about to be wiped.
    pub fn teardown(&self) {
        let _order = self.order();
        if let Some(mut region) = self.region().take() {
            region.erase();
        }
        // A run that exits while a `tty:` task still owns the terminal has held
        // output nobody else will ever flush: the task body that would hand
        // ownership back is about to be dropped, or already was. Teardown is
        // the last writer, so it is also the last chance.
        let held = self.terminal().take();
        if let Some(held) = held {
            let stdout = io::stdout();
            let stderr = io::stderr();
            held.drain_into(&mut stdout.lock(), &mut stderr.lock());
        }
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
    }

    /// Hand the terminal to a `tty:` task, holding otto's own writes until it
    /// is handed back.
    ///
    /// Taking the ordering lock is the handoff itself: it cannot return while
    /// another writer is mid-block, so the child is never spawned into the
    /// middle of a replay. `spill_path` is where held bytes go past the memory
    /// bound, and belongs to the caller because only it knows the task's run
    /// directory.
    ///
    /// The live region is erased here, before the child can draw over it, and
    /// re-armed by [`Facade::reclaim`]. Driven by Phase 4's admission gate: by
    /// the time a `tty:` task is admitted nothing else is in flight, so the
    /// rows this erases are the last of them.
    pub fn surrender(&self, task: &str, spill_path: PathBuf) -> TerminalHandoff<'_> {
        let _order = self.order();
        let mut terminal = self.terminal();
        if terminal.is_some() {
            // Unreachable under the admission gate, which admits a `tty:` task
            // only when nothing else is in flight. Loud and harmless rather
            // than a second guard that would hand the terminal back early.
            log::error!("surrender: {task} found the terminal already surrendered; not taking it");
            return TerminalHandoff::new(self, false);
        }
        debug!("surrender: task={task} spill={}", spill_path.display());
        if let Some(region) = self.region().as_mut() {
            region.erase();
        }
        *terminal = Some(Surrendered::new(task, spill_path));
        TerminalHandoff::new(self, true)
    }

    /// Take the terminal back and flush everything held while it was gone.
    ///
    /// Called from [`TerminalHandoff`]'s `Drop`, so it runs on the normal reap,
    /// on a spawn that failed, and on a cancelled run whose body is dropped.
    ///
    /// The region is re-armed here, AFTER the flush: held output is scrollback
    /// and belongs above the rows, not underneath them.
    pub(super) fn reclaim(&self) {
        let _order = self.order();
        let held = self.terminal().take();
        let Some(held) = held else {
            return;
        };
        self.reset_attributes();
        let stdout = io::stdout();
        let stderr = io::stderr();
        {
            let mut out = TrackedWriter {
                inner: &mut stdout.lock(),
                facade: self,
                stream: Stream::Stdout,
            };
            let mut err = TrackedWriter {
                inner: &mut stderr.lock(),
                facade: self,
                stream: Stream::Stderr,
            };
            held.drain_into(&mut out, &mut err);
        }
        if let Some(region) = self.region().as_mut() {
            self.ensure_line_start(REGION_STREAM);
            region.rearm();
        }
    }

    /// Close any SGR state a `tty:` child left open, before otto writes a byte
    /// of its own.
    ///
    /// The design's handoff diagram requires it on reclaim, and it is not
    /// decorative: a child that exits with `\x1b[31m` still active tints every
    /// byte otto writes for the rest of the run. It went unnoticed because
    /// otto's own coloured label happens to carry a reset; under `NO_COLOR` it
    /// carries none and the bleed is total.
    ///
    /// Only onto a terminal, and never through [`Facade::note_written`]: a
    /// reset moves no cursor, so it changes no line state, and a captured
    /// stream has no attributes to reset and must not gain escape bytes.
    fn reset_attributes(&self) {
        if !io::stderr().is_terminal() {
            return;
        }
        raw_write(REGION_STREAM, "\x1b[0m");
    }

    /// Take what is currently held without writing it anywhere.
    ///
    /// [`Facade::reclaim`]'s own first step, minus the write, so a test can
    /// assert what a surrender captured without defacing the harness's
    /// terminal. Not in the shipped binary: reclaim keeps the take and the
    /// write under ONE acquisition of the ordering lock, and splitting them to
    /// share this would let another writer land between them.
    #[cfg(test)]
    pub(super) fn take_held(&self) -> Option<Surrendered> {
        let _order = self.order();
        self.terminal().take()
    }
}

/// Whether otto's own stdout and stderr land on the same open destination.
///
/// `fstat` on both handles and compare the device, inode and rdev: one
/// terminal answers the same triple twice, `> log 2>&1` answers the same
/// triple twice, and two pipes, two files, or a file and a terminal do not.
/// That is exactly the question the facade's line state has to answer, and it
/// is a question about the invocation rather than about the bytes, so it is
/// asked once.
///
/// A handle that cannot be `fstat`ed is treated as NOT shared: the cost of
/// guessing wrong that way is a missing newline on one line, and the cost of
/// guessing wrong the other way is a spurious newline in a captured stream,
/// which is the regression this replaces.
fn streams_share_destination() -> bool {
    fn identity(fd: i32) -> Option<(libc::dev_t, libc::ino_t, libc::dev_t)> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // Safe: `fstat` writes the whole `struct stat` on success and touches
        // nothing else, and the fd is one of the process's own standard
        // handles.
        let rc = unsafe { libc::fstat(fd, stat.as_mut_ptr()) };
        if rc != 0 {
            return None;
        }
        let stat = unsafe { stat.assume_init() };
        Some((stat.st_dev, stat.st_ino, stat.st_rdev))
    }
    match (identity(libc::STDOUT_FILENO), identity(libc::STDERR_FILENO)) {
        (Some(out), Some(err)) => out == err,
        _ => false,
    }
}

/// One best-effort write to one of otto's own handles.
///
/// Flushed per call, never panicking on failure: a run that ended because the
/// terminal hung up has a stderr whose every write fails `EIO`, and there is
/// nowhere left to report that to.
fn raw_write(stream: Stream, bytes: &str) {
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

/// A `Write` that keeps the facade's newline tracking honest about bytes that
/// went out through a locked handle rather than through [`Facade::write`].
///
/// Replay writes whole log files this way, and a log whose last line is
/// unterminated leaves the cursor mid-line exactly like a live final chunk
/// does.
struct TrackedWriter<'a> {
    inner: &'a mut dyn Write,
    facade: &'a Facade,
    stream: Stream,
}

impl Write for TrackedWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.facade.note_written(self.stream, &buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A `Write` that holds bytes for a surrendered terminal instead of sending
/// them to one.
///
/// Both of a block's streams point at the same sink, which is what preserves
/// the interleave replay depends on.
struct HeldWriter<'a, 'b> {
    stream: Stream,
    sink: &'a RefCell<&'b mut Surrendered>,
}

impl Write for HeldWriter<'_, '_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sink.borrow_mut().push(self.stream, buf);
        Ok(buf.len())
    }

    /// Nothing to flush: the bytes are already as far along as they go until
    /// the terminal comes back.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[path = "facade_tests.rs"]
mod tests;
