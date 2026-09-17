//! Phase 0 spike for `docs/design/2026-09-16-live-progress-renderer.md`: what
//! indicatif 0.18.3 will and will not do for a stream that is not a terminal.
//!
//! Zero production code. These are the measurements behind two Phase 0
//! verdicts, kept as tests rather than as a paragraph so a later bump cannot
//! quietly invalidate them:
//!
//! - **`--progress always` is CUT.** `ProgressDrawTarget::term_like` does draw
//!   through a non-terminal sink, exactly as the design says. What otto cannot
//!   do is supply the rest: there is no width, no height, and no working cursor
//!   for a stream nobody is looking at, so every "redraw" appends another whole
//!   frame. The design's own cut condition, verbatim: "What must be proven is
//!   that otto can supply correct `width`, `height`, and cursor operations for
//!   a stream that is not a terminal. If it cannot, `always` is cut and the
//!   flag ships as `auto | never`."
//! - **Stay on indicatif 0.18.3.** otto owns the `TERM=dumb` policy itself.
//!
//! The bars here are built directly, not through otto, because the seam under
//! test is indicatif's and otto has no renderer yet.

use console::Term;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle, TermLike};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// What a `TermLike` over a pipe can actually be: a byte sink, plus whatever
/// width and height otto decides to claim.
///
/// The cursor operations write their ANSI spelling into the same sink, which is
/// the honest model of a pipe: a cursor movement is not an action there, it is
/// four more bytes in the stream. `ops` records the calls separately so a test
/// can assert indicatif tried to move the cursor at all.
#[derive(Debug, Clone)]
struct SinkTerm {
    width: u16,
    height: u16,
    bytes: Arc<Mutex<Vec<u8>>>,
    ops: Arc<Mutex<Vec<String>>>,
}

impl SinkTerm {
    fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            bytes: Arc::new(Mutex::new(Vec::new())),
            ops: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn written(&self) -> String {
        String::from_utf8_lossy(&self.bytes.lock().unwrap()).into_owned()
    }

    fn byte_len(&self) -> usize {
        self.bytes.lock().unwrap().len()
    }

    fn ops(&self) -> Vec<String> {
        self.ops.lock().unwrap().clone()
    }

    fn record(&self, op: &str, ansi: &str) -> io::Result<()> {
        self.ops.lock().unwrap().push(op.to_string());
        self.bytes.lock().unwrap().extend_from_slice(ansi.as_bytes());
        Ok(())
    }
}

impl TermLike for SinkTerm {
    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }

    fn move_cursor_up(&self, n: usize) -> io::Result<()> {
        self.record("move_cursor_up", &format!("\x1b[{n}A"))
    }

    fn move_cursor_down(&self, n: usize) -> io::Result<()> {
        self.record("move_cursor_down", &format!("\x1b[{n}B"))
    }

    fn move_cursor_right(&self, n: usize) -> io::Result<()> {
        self.record("move_cursor_right", &format!("\x1b[{n}C"))
    }

    fn move_cursor_left(&self, n: usize) -> io::Result<()> {
        self.record("move_cursor_left", &format!("\x1b[{n}D"))
    }

    fn write_line(&self, s: &str) -> io::Result<()> {
        self.ops.lock().unwrap().push("write_line".to_string());
        let mut bytes = self.bytes.lock().unwrap();
        bytes.extend_from_slice(s.as_bytes());
        bytes.push(b'\n');
        Ok(())
    }

    fn write_str(&self, s: &str) -> io::Result<()> {
        self.ops.lock().unwrap().push("write_str".to_string());
        self.bytes.lock().unwrap().extend_from_slice(s.as_bytes());
        Ok(())
    }

    fn clear_line(&self) -> io::Result<()> {
        self.record("clear_line", "\r\x1b[2K")
    }

    fn flush(&self) -> io::Result<()> {
        Ok(())
    }
}

/// A row in the shape the design draws: `{wide_msg}` with no bar, no spinner.
fn row(mp: &MultiProgress, msg: &str) -> ProgressBar {
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_style(ProgressStyle::with_template("{wide_msg}").expect("static template"));
    pb.set_message(msg.to_string());
    pb
}

// ---------------------------------------------------------------------------
// Verdict 1: `--progress always`.
// ---------------------------------------------------------------------------

/// The design's claim about `draw_target.rs:198`, confirmed: the `TermLike`
/// draw branch carries no terminal predicate, so a `TermLike` over a pipe
/// draws. Nothing has to claim to be a terminal; the trait has no such method.
///
/// This half of the design is RIGHT. The three tests after it are why `always`
/// is still cut.
#[test]
fn term_like_draws_through_a_non_terminal_sink() {
    let sink = SinkTerm::new(80, 24);
    let mp = MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(sink.clone())));
    let a = row(&mp, "philo    running 4m05s   no output for 41s");
    let b = row(&mp, "api      running 1m12s");
    a.tick();
    b.tick();

    let written = sink.written();
    assert!(
        written.contains("philo    running 4m05s") && written.contains("api      running 1m12s"),
        "term_like drew nothing into a non-terminal sink:\n{written:?}"
    );
    assert!(
        sink.ops().iter().any(|op| op == "move_cursor_up"),
        "indicatif never tried to reposition the cursor, so this is not measuring a redraw: {:?}",
        sink.ops()
    );
}

/// The other side of the same branch, and the reason `always` cannot simply set
/// `ProgressMode::Live`: a plain `Term` target returns `None` from `drawable`
/// on `!term.is_term()` (`draw_target.rs:176`) before it ever consults
/// `force_draw` (`:180`).
///
/// Measured through `Term::read_write_pair` over a regular file, which is a
/// non-terminal fd by construction and does not depend on how the test harness
/// wired its own stdio.
#[test]
fn a_plain_term_target_over_a_non_terminal_never_draws() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("sink.log");
    let write = File::create(&path).unwrap();
    let read = File::open("/dev/null").unwrap();
    let term = Term::read_write_pair(read, write);
    assert!(!term.is_term(), "the fixture must not be a terminal");

    let target = ProgressDrawTarget::term(term, 20);
    assert!(
        target.is_hidden(),
        "a Term target over a non-terminal must report hidden"
    );

    let pb = ProgressBar::with_draw_target(None, target);
    pb.set_style(ProgressStyle::with_template("{wide_msg}").expect("static template"));
    pb.set_message("philo    running 4m05s");
    for _ in 0..10 {
        pb.tick();
    }
    pb.finish_and_clear();

    let written = std::fs::read(&path).unwrap();
    assert!(
        written.is_empty(),
        "a Term target wrote {} bytes into a non-terminal: {:?}",
        written.len(),
        String::from_utf8_lossy(&written)
    );
}

/// Cut reason 1: otto cannot learn the size of a stream that is not a terminal.
///
/// `TIOCGWINSZ` is the only source of truth and it is an ioctl on a terminal
/// fd. console asks it and gives up honestly (`size_checked` -> `None`,
/// `console-0.16.2/src/unix_term.rs:55-58`); `size()` then invents `(24, 80)`
/// (`term.rs:421-423`). Inventing a width is exactly what the design forbids
/// elsewhere: rows are truncated to `width - 1` and a wrong width is
/// wrap-miscount drift with nothing to correct it against.
#[test]
fn otto_cannot_learn_the_size_of_a_non_terminal_stream() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("sink.log");
    let write = File::create(&path).unwrap();

    // The ground truth under console's answer: the ioctl itself fails.
    let mut winsize: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(write.as_raw_fd(), libc::TIOCGWINSZ, &mut winsize) };
    assert_eq!(
        rc, -1,
        "TIOCGWINSZ unexpectedly succeeded on a regular file; this test is not measuring a pipe"
    );

    let term = Term::read_write_pair(File::open("/dev/null").unwrap(), write);
    assert_eq!(
        term.size_checked(),
        None,
        "console claimed to know the size of a non-terminal"
    );
    assert_eq!(
        term.size(),
        (24, 80),
        "console's non-terminal fallback moved; the invented size is the thing being recorded"
    );
}

/// Cut reason 2: a cursor operation into a stream nobody is looking at is not
/// an operation, it is four more bytes. Every redraw appends a whole frame.
///
/// This is what `--progress always` would actually put in `build.log`: N copies
/// of the region for N ticks, with the erase sequences interleaved, growing
/// without bound for the length of the run. The design's stated goal is "zero
/// new bytes in a captured stream"; `always` is the deliberate opt-out from
/// that, but the opt-out has to be a region, not a transcript of one.
///
/// Measured 2026-09-16 on indicatif 0.18.3, ONE row, 12 redraws: 181 bytes
/// after the first frame, 2413 after the twelfth, 26 copies of the row text.
/// A run with K rows and a 20 Hz ticker scales both numbers by K and by
/// duration.
#[test]
fn a_redraw_through_a_non_terminal_sink_appends_a_whole_frame_each_time() {
    const REDRAWS: usize = 12;

    let sink = SinkTerm::new(80, 24);
    let mp = MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(sink.clone())));
    let pb = row(&mp, "philo    running 0m00s");
    pb.tick();
    let after_first = sink.byte_len();

    for i in 1..=REDRAWS {
        pb.set_message(format!("philo    running 0m{i:02}s"));
        pb.tick();
    }

    let written = sink.written();
    let frames = written.matches("philo    running").count();
    assert!(
        frames >= REDRAWS,
        "expected at least one frame per redraw, got {frames} for {REDRAWS} redraws; \
         if indicatif learned to update in place through a pipe, this verdict is stale"
    );
    assert!(
        sink.byte_len() > after_first * REDRAWS / 2,
        "the sink did not grow with the redraws ({} bytes after {REDRAWS} redraws vs {after_first} \
         after the first), so this is no longer measuring unbounded growth",
        sink.byte_len()
    );
}

/// Cut reason 3, and a trap for any later phase that reaches for `term_like`
/// anyway: `ProgressDrawTarget::term_like` installs `rate_limiter: None`
/// (`draw_target.rs:84-93`), so EVERY tick draws. `term()` and `stderr()` both
/// take a refresh rate; `term_like` silently does not, and 0.18.6 added a
/// doc-comment warning about it rather than a default.
///
/// The design does not mention this anywhere. Combined with the frame-per-
/// redraw property above, `always` into a file writes one full region per tick
/// at whatever rate the ticker runs.
///
/// Measured 2026-09-16, 100 ticks of one row: 101 frames through `term_like`,
/// 20 through `term_like_with_hz(_, 1)`. The throttled 20 is the limiter's
/// initial burst capacity (`MAX_BURST`, `draw_target.rs:484`), after which it
/// stops; the unthrottled case never stops.
#[test]
fn term_like_installs_no_rate_limiter_while_term_like_with_hz_does() {
    // Above `MAX_BURST` (`draw_target.rs:484`, 20), or the rate limiter's
    // initial burst capacity covers the whole sample and the two cases measure
    // the same thing.
    const TICKS: usize = 100;

    let unthrottled = SinkTerm::new(80, 24);
    let mp = MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(unthrottled.clone())));
    let pb = row(&mp, "row");
    for _ in 0..TICKS {
        pb.tick();
    }
    let unthrottled_frames = unthrottled.written().matches("row").count();

    let throttled = SinkTerm::new(80, 24);
    let mp = MultiProgress::with_draw_target(ProgressDrawTarget::term_like_with_hz(Box::new(throttled.clone()), 1));
    let pb = row(&mp, "row");
    for _ in 0..TICKS {
        pb.tick();
    }
    let throttled_frames = throttled.written().matches("row").count();

    assert!(
        unthrottled_frames >= TICKS,
        "term_like throttled something: {unthrottled_frames} frames for {TICKS} ticks"
    );
    assert!(
        throttled_frames < unthrottled_frames,
        "term_like_with_hz(1) drew {throttled_frames} frames and term_like drew \
         {unthrottled_frames}; the missing default refresh rate is the finding"
    );
}

// ---------------------------------------------------------------------------
// Verdict 2: stay on indicatif 0.18.3.
// ---------------------------------------------------------------------------

/// The verdict, made enforceable.
///
/// `Cargo.toml` says `indicatif = "0.18"`, so nothing but `Cargo.lock` holds
/// otto at the version every measurement in this file was taken against. A
/// `cargo update` would move it to 0.18.6 silently, and 0.18.6 is not a
/// drop-in: it requires `console >= 0.16.4` (`indicatif-0.18.6/Cargo.toml:142`)
/// against the locked 0.16.2, so the bump is two crates.
///
/// What the bump buys, measured from the published sources: `is_dumb()`, and
/// only inside `ProgressDrawTarget::term()` (`draw_target.rs:80`). The
/// `term_like` path has no dumb check in either version, and `auto` has to read
/// `CI` regardless, which no version of indicatif knows about. One predicate in
/// one otto-owned place beats two sources of truth for whether to draw, which
/// is the npm `5b858c6` failure the design already cites. So otto owns the
/// `TERM=dumb` policy and the lock stays.
///
/// Recorded against the bump, for Phase 5 rather than for this decision: on
/// 0.18.3 `is_stderr()` returns `false` for `TargetKind::Multi`
/// (`draw_target.rs:133-140`), fixed in 0.18.6 (`:150`). Every row inside a
/// `MultiProgress` is therefore styled with console's STDOUT colour decision
/// even when the region is on stderr, which is the same class of bug otto fixed
/// at `output.rs:192-194`. Phase 5 must set row styles explicitly instead of
/// relying on indicatif's auto-targeting.
///
/// Changing the version below is allowed. Changing it without re-running this
/// file is not, which is why it is an assertion and not a comment.
#[test]
fn the_lock_pins_the_indicatif_and_console_this_spike_measured() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).expect("Cargo.lock");

    for (crate_name, version) in [("indicatif", "0.18.3"), ("console", "0.16.2")] {
        let needle = format!("name = \"{crate_name}\"\nversion = \"{version}\"");
        assert!(
            lock.contains(&needle),
            "Cargo.lock no longer pins {crate_name} {version}. Phase 0's measurements \
             (tests/progress_spike_indicatif_test.rs) were taken against that version and its \
             verdicts were chosen from them. Re-run this file and update the verdict before \
             changing this line."
        );
    }
}

// ---------------------------------------------------------------------------
// The chatty-path benchmark the design's Performance section assigns to Phase 0
// ("Phase 0 benchmarks the chatty path before K is chosen"). The Phase 0
// section's own bullets and success criteria never mention it; it is measured
// here anyway, because K cannot be chosen without it.
//
// Counted in bytes and frames rather than wall time: `mp.suspend` "forces a
// redraw after every callback, bypassing indicatif's draw throttle" is a claim
// about how much gets written, and a byte count is the same number on a loaded
// machine as on an idle one.
// ---------------------------------------------------------------------------

/// Per-line suspension costs one full region redraw per line; one suspension
/// for a whole block costs one. This is the facade's batching rule, measured.
///
/// Measured 2026-09-16, 10 rows, 200 chatty lines, indicatif 0.18.3: per-line
/// suspension wrote 82,290 bytes and drew the region 220 times; one suspension
/// for the whole block wrote 8,461 bytes and drew it 21 times. Both byte counts
/// include the ~8 KiB of chatty lines themselves, so the region-only cost is
/// roughly 74 KiB against 0.5 KiB.
///
/// What it says about K: the per-line region cost is linear in K, so the same
/// 200 lines against `K = max(3, term_height - 6)` on an 80x50 terminal (K =
/// 44) is ~4.4x this measurement. The design's rule that replay holds ONE
/// suspension for a whole block, never one per line, is what keeps K out of the
/// chatty path's cost entirely.
#[test]
fn per_line_suspension_costs_a_full_region_redraw_for_every_line() {
    const ROWS: usize = 10;
    const LINES: usize = 200;

    fn drive(per_line: bool) -> (usize, usize) {
        let sink = SinkTerm::new(80, 24);
        let mp = MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(sink.clone())));
        let bars: Vec<_> = (0..ROWS)
            .map(|i| row(&mp, &format!("task{i}  running 0m01s")))
            .collect();
        for bar in &bars {
            bar.tick();
        }
        let baseline = sink.byte_len();
        let emit = |n: usize| {
            for i in 0..n {
                let _ = sink.write_line(&format!("[philo] pg_restore: processing chunk {i}"));
            }
        };
        if per_line {
            for i in 0..LINES {
                mp.suspend(|| {
                    let _ = sink.write_line(&format!("[philo] pg_restore: processing chunk {i}"));
                });
            }
        } else {
            mp.suspend(|| emit(LINES));
        }
        (
            sink.byte_len() - baseline,
            sink.written().matches("task0  running").count(),
        )
    }

    let (per_line_bytes, per_line_frames) = drive(true);
    let (per_block_bytes, per_block_frames) = drive(false);

    assert!(
        per_line_frames > per_block_frames * 5,
        "per-line suspension drew {per_line_frames} frames and per-block drew {per_block_frames}; \
         if those are close, `mp.suspend` stopped forcing a redraw and the facade's batching rule \
         needs re-deriving"
    );
    assert!(
        per_line_bytes > per_block_bytes * 5,
        "per-line suspension wrote {per_line_bytes} bytes and per-block wrote {per_block_bytes}"
    );
}
