//! Terminal ownership for `tty:` tasks.
//!
//! A `tty: true` task inherits otto's own stdout and stderr and writes outside
//! every otto lock (`task_execution.rs`, the `tty` arm of the run block). Phase
//! 4 of `docs/design/2026-09-16-live-progress-renderer.md` turns that collision
//! into a handoff: otto surrenders the terminal for the child's lifetime and
//! takes it back when the child is reaped.
//!
//! While surrendered, otto's own writes are HELD, not dropped: a status line or
//! a failure message that arrives in that window still has to reach the user.
//! They are held under a bound, though. Buffering a whole buffered-foreach
//! replay as bytes would trade replay's 64 KiB streaming bound
//! (`scheduler/replay.rs`) for the full size of a log, for as long as an
//! interactive child runs. Past [`SURRENDER_MEMORY_BYTES`] the bytes go to a
//! file in the tty task's own run directory and are re-streamed from there on
//! re-arm.
//!
//! What they must never do is spill to the streaming path: that path takes the
//! real stdout/stderr handles, and writing those while a `tty:` child owns the
//! terminal is the exact collision the surrender exists to prevent.
//!
//! Phase 4 ships the handoff with no renderer behind it. Phase 5 adds the
//! region erase and the re-arm redraw inside `Facade::surrender` and
//! `Facade::reclaim`; no caller changes when it does.

use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
};

use log::{debug, warn};

use super::facade::{Facade, Stream};

/// How many bytes of otto's own output are held in memory while a `tty:` task
/// owns the terminal.
///
/// Matched to replay's own streaming bound rather than picked fresh: the
/// largest thing that can arrive here is a replayed block, which reaches the
/// facade one 64 KiB chunk at a time.
pub const SURRENDER_MEMORY_BYTES: usize = 64 * 1024;

/// Stream tags written ahead of every spilled record.
const SPILL_TAG_STDOUT: u8 = 0;
const SPILL_TAG_STDERR: u8 = 1;

/// Bytes of framing per spilled record: one stream tag plus a `u64` length.
const SPILL_HEADER_BYTES: usize = 9;

/// One held write, tagged with the stream it was headed for.
///
/// Segments rather than two per-stream buffers: replay interleaves a subtask's
/// stdout and stderr inside one block, and re-arm has to put those bytes back
/// in the order they were written, not stream by stream.
struct Segment {
    stream: Stream,
    bytes: Vec<u8>,
}

/// What otto wrote while a `tty:` task owned the terminal, in order.
///
/// Built when the terminal is surrendered and consumed exactly once, by
/// [`Surrendered::drain_into`] on re-arm.
///
/// Three ordered stages, drained in this order, because a spill that fails
/// halfway must not silently reorder what came after it:
///
/// 1. `held` - everything up to the memory bound.
/// 2. the spill file - everything past it, framed.
/// 3. `overflow` - everything after the spill itself broke.
pub(super) struct Surrendered {
    /// The `tty:` task that owns the terminal, so a warning about a failed
    /// spill can name whose window it happened in.
    task: String,
    /// Where bytes past the memory bound go. Inside the tty task's own run
    /// directory, which is the one destination guaranteed not to be a terminal.
    spill_path: PathBuf,
    held: Vec<Segment>,
    held_bytes: usize,
    /// Set the first time the spill file is opened. Kept separate from `spill`
    /// so a write failure that closes the handle cannot lose the records
    /// already on disk.
    spill_started: bool,
    spill: Option<fs::File>,
    /// Bytes written after the spill broke. Unbounded by construction, which is
    /// the deliberate choice: losing a failure status is worse than exceeding a
    /// buffer policy on a filesystem that is already failing.
    overflow: Vec<Segment>,
    /// Why the spill could not be opened or written, if it could not be.
    spill_error: Option<String>,
}

/// Append `bytes` to `segments`, coalescing with the previous write when it
/// went to the same stream. Returns how many bytes were added.
fn append(segments: &mut Vec<Segment>, stream: Stream, bytes: &[u8]) -> usize {
    match segments.last_mut() {
        Some(last) if last.stream == stream => last.bytes.extend_from_slice(bytes),
        _ => segments.push(Segment {
            stream,
            bytes: bytes.to_vec(),
        }),
    }
    bytes.len()
}

impl Surrendered {
    pub(super) fn new(task: &str, spill_path: PathBuf) -> Self {
        Self {
            task: task.to_string(),
            spill_path,
            held: Vec::new(),
            held_bytes: 0,
            spill_started: false,
            spill: None,
            overflow: Vec::new(),
            spill_error: None,
        }
    }

    /// Bytes held in memory before the spill. The bound this type exists to
    /// keep, and nothing in the shipped binary asks: only a test can tell the
    /// difference between honouring it and not.
    #[cfg(test)]
    pub(super) fn held_bytes(&self) -> usize {
        self.held_bytes
    }

    /// Whether anything has reached the spill file. Test-only, for the same
    /// reason [`Surrendered::held_bytes`] is.
    #[cfg(test)]
    pub(super) fn spilled(&self) -> bool {
        self.spill_started
    }

    /// Hold one write.
    ///
    /// Once anything has spilled, everything spills: memory and file are one
    /// ordered sequence, and putting a later write in front of an earlier one
    /// would reorder the terminal on re-arm.
    pub(super) fn push(&mut self, stream: Stream, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if !self.spill_started && self.held_bytes + bytes.len() <= SURRENDER_MEMORY_BYTES {
            self.held_bytes += append(&mut self.held, stream, bytes);
            return;
        }
        self.spill_write(stream, bytes);
    }

    /// Append one framed record to the spill file, opening it on first use.
    ///
    /// Framing is `tag | len | bytes` because the two streams interleave: a
    /// plain concatenation could not be put back on the right handle.
    fn spill_write(&mut self, stream: Stream, bytes: &[u8]) {
        if self.spill_error.is_some() {
            append(&mut self.overflow, stream, bytes);
            return;
        }
        if self.spill.is_none() {
            debug!("spill_write: task={} opening {}", self.task, self.spill_path.display());
            match fs::File::create(&self.spill_path) {
                Ok(file) => {
                    self.spill = Some(file);
                    self.spill_started = true;
                }
                Err(e) => {
                    self.spill_error = Some(format!("could not create {}: {e}", self.spill_path.display()));
                    append(&mut self.overflow, stream, bytes);
                    return;
                }
            }
        }
        let tag = match stream {
            Stream::Stdout => SPILL_TAG_STDOUT,
            Stream::Stderr => SPILL_TAG_STDERR,
        };
        let file = self.spill.as_mut().expect("opened above or already present");
        let written = file
            .write_all(&[tag])
            .and_then(|()| file.write_all(&(bytes.len() as u64).to_le_bytes()))
            .and_then(|()| file.write_all(bytes));
        if let Err(e) = written {
            // The handle goes, the records already on disk stay: `spill_started`
            // is what drives the re-stream, so they are not lost with it.
            self.spill = None;
            self.spill_error = Some(format!("could not write {}: {e}", self.spill_path.display()));
            append(&mut self.overflow, stream, bytes);
        }
    }

    /// Put everything back on the terminal, in the order it was written, and
    /// remove the spill file.
    ///
    /// Takes both handles because the held segments name both streams. Every
    /// write is best effort, matching the facade's own contract: there is
    /// nowhere left to report a failed terminal write to.
    pub(super) fn drain_into(mut self, out: &mut dyn Write, err: &mut dyn Write) {
        debug!(
            "drain_into: task={} held_bytes={} spilled={} overflow={}",
            self.task,
            self.held_bytes,
            self.spill_started,
            self.overflow.len()
        );
        // Closed before the file is re-read, so a buffered tail is on disk.
        self.spill = None;

        write_segments(self.held.drain(..), out, err);
        if self.spill_started {
            self.drain_spill(out, err);
        }
        write_segments(self.overflow.drain(..), out, err);

        if let Some(reason) = &self.spill_error {
            let _ = err.write_all(
                format!(
                    "otto: WARNING: output held while {} owned the terminal could not be spilled: {reason}\n",
                    self.task
                )
                .as_bytes(),
            );
            let _ = err.flush();
        }
    }

    /// Re-stream the spill file's framed records, then delete it.
    ///
    /// A record is copied in pieces rather than read whole, so a spilled 100 MB
    /// replay costs the copy buffer and not its own size.
    fn drain_spill(&self, out: &mut dyn Write, err: &mut dyn Write) {
        let file = match fs::File::open(&self.spill_path) {
            Ok(file) => file,
            Err(e) => {
                let _ = err.write_all(
                    format!(
                        "otto: WARNING: held output at {} could not be re-read: {e}\n",
                        self.spill_path.display()
                    )
                    .as_bytes(),
                );
                let _ = err.flush();
                return;
            }
        };
        let mut reader = io::BufReader::new(file);
        let mut header = [0u8; SPILL_HEADER_BYTES];
        loop {
            match reader.read_exact(&mut header) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    let _ = err.write_all(format!("otto: WARNING: held output ended early: {e}\n").as_bytes());
                    let _ = err.flush();
                    break;
                }
            }
            let len = u64::from_le_bytes(header[1..].try_into().expect("eight bytes after the tag"));
            let sink: &mut dyn Write = if header[0] == SPILL_TAG_STDOUT { out } else { err };
            if io::copy(&mut (&mut reader).take(len), sink).is_err() {
                break;
            }
            let _ = sink.flush();
        }
        if let Err(e) = fs::remove_file(&self.spill_path) {
            warn!("drain_spill: could not remove {}: {e}", self.spill_path.display());
        }
    }
}

/// Write held segments to the handle each one names, best effort, flushing
/// both at the end.
fn write_segments(segments: impl Iterator<Item = Segment>, out: &mut dyn Write, err: &mut dyn Write) {
    for segment in segments {
        let sink: &mut dyn Write = match segment.stream {
            Stream::Stdout => &mut *out,
            Stream::Stderr => &mut *err,
        };
        let _ = sink.write_all(&segment.bytes);
    }
    let _ = out.flush();
    let _ = err.flush();
}

/// Otto's claim on the terminal, handed back when this is dropped.
///
/// A guard rather than a pair of calls, because the release has to happen on
/// every way out of the spawn: a normal reap, a spawn that failed, and a
/// cancelled run whose task body is dropped mid-await. Only the first of those
/// reaches a line an author could have written a release call on.
pub struct TerminalHandoff<'a> {
    facade: &'a Facade,
    /// False when the facade was already surrendered to another task, which is
    /// unreachable under Phase 4's admission gate. A guard that did not take
    /// ownership must not hand back ownership it never had.
    owns: bool,
}

impl<'a> TerminalHandoff<'a> {
    pub(super) fn new(facade: &'a Facade, owns: bool) -> Self {
        Self { facade, owns }
    }
}

impl Drop for TerminalHandoff<'_> {
    fn drop(&mut self) {
        if self.owns {
            self.facade.reclaim();
        }
    }
}

#[path = "ownership_tests.rs"]
mod tests;
