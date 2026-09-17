#![cfg(test)]

use std::{
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use super::{Facade, ProgressMode, Stream, facade, reset_targets, share_destination};
use crate::executor::clocks::TaskClocks;

/// The whole reason the facade owns a lock: two callers cannot be inside a
/// block at the same time, so a replayed block cannot be split.
#[test]
fn a_block_excludes_every_other_block() {
    let f = Arc::new(Facade::new());
    let inside = Arc::new(AtomicBool::new(false));
    let overlaps = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let f = Arc::clone(&f);
            let inside = Arc::clone(&inside);
            let overlaps = Arc::clone(&overlaps);
            std::thread::spawn(move || {
                for _ in 0..50 {
                    f.block(|_| {
                        if inside.swap(true, Ordering::SeqCst) {
                            overlaps.fetch_add(1, Ordering::SeqCst);
                        }
                        std::thread::yield_now();
                        inside.store(false, Ordering::SeqCst);
                    });
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("no thread should panic");
    }

    assert_eq!(
        overlaps.load(Ordering::SeqCst),
        0,
        "two threads were inside a block at once, so the ordering invariant is gone"
    );
}

/// A write is ordered against a block too, not only against another write:
/// `TeeWriter`'s per-line writes are exactly what must not land mid-replay.
///
/// Asserted from the safe direction. A write CANNOT complete while a block is
/// open, so a failure here is always a real one; the worst a scheduling hiccup
/// can do is let the test pass without having raced.
#[test]
fn a_write_cannot_complete_while_a_block_is_open() {
    let f = Arc::new(Facade::new());
    let reached = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));

    let writer = f.block(|_| {
        let thread_facade = Arc::clone(&f);
        let thread_reached = Arc::clone(&reached);
        let thread_finished = Arc::clone(&finished);
        let writer = std::thread::spawn(move || {
            thread_reached.store(true, Ordering::SeqCst);
            // Empty bytes on purpose: this measures the lock, not the
            // terminal, and a test must not deface the harness's output.
            thread_facade.write(Stream::Stdout, "");
            thread_finished.store(true, Ordering::SeqCst);
        });

        while !reached.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !finished.load(Ordering::SeqCst),
            "a write completed while a block was open, so a status line can still split a replayed block"
        );

        writer
    });

    writer.join().expect("the write completes once the block closes");
    assert!(
        finished.load(Ordering::SeqCst),
        "the write never completed after the block closed, so the lock was not released"
    );
}

/// Replay needs BOTH streams under one lock and needs its own value back out;
/// a single `&mut dyn Write` cannot express the interleave.
#[test]
fn a_block_hands_back_both_streams_and_the_closures_value() {
    let f = Facade::new();
    let reached = f.block(|streams| {
        // Zero-length writes: the point is that both handles are usable inside
        // one lock, not that anything reaches the terminal.
        streams.out.write_all(b"").expect("stdout handle is usable");
        streams.err.write_all(b"").expect("stderr handle is usable");
        streams.out.flush().expect("stdout handle flushes");
        streams.err.flush().expect("stderr handle flushes");
        "both streams"
    });
    assert_eq!(
        reached, "both streams",
        "the closure's value must come back out of the block; replay reports what it wrote"
    );
}

/// The single reason otto keeps its own lock instead of letting indicatif own
/// one: indicatif unwraps a poisoned lock, so one panicking writer would mute
/// the rest of the run.
#[test]
fn a_poisoned_ordering_lock_is_recovered_not_propagated() {
    let f = Arc::new(Facade::new());

    let poisoner = {
        let f = Arc::clone(&f);
        std::thread::spawn(move || {
            let _order = f.order();
            panic!("deliberate: poisoning the ordering lock so the recovery path is exercised");
        })
    };
    assert!(poisoner.join().is_err(), "the poisoning thread was supposed to panic");
    assert!(f.order.is_poisoned(), "the lock did not actually get poisoned");

    // Neither of these may panic, and neither may hang.
    f.write(Stream::Stderr, "");
    f.block(|_| {});
    f.teardown();
}

// ---------------------------------------------------------------------------
// Line state, per stream.
// ---------------------------------------------------------------------------

/// One flag for two streams was wrong in both directions. The measured
/// regression: `printf ERR >&2` with both streams captured to separate files
/// put a LEADING newline into stdout, because the unterminated chunk on stderr
/// cleared the one shared flag and the completion line on stdout then
/// "corrected" a cursor that was already at column zero.
#[test]
fn an_unterminated_chunk_on_one_stream_leaves_the_other_at_line_start() {
    let f = Facade::with_one_destination(false);
    f.note_written(Stream::Stderr, b"ERR");
    assert!(
        !f.line_start(Stream::Stderr),
        "stderr is mid-line after an unterminated chunk"
    );
    assert!(
        f.line_start(Stream::Stdout),
        "a write to stderr moved no cursor on a stdout that is somewhere else"
    );
}

/// And the other half: when the two handles ARE one destination - one terminal,
/// or one file both were redirected to - they share a cursor, so they share a
/// line state. This is the case the single flag got right, and it has to keep
/// working.
#[test]
fn two_handles_on_one_destination_share_one_line_state() {
    let f = Facade::with_one_destination(true);
    f.note_written(Stream::Stdout, b"DONE");
    assert!(
        !f.line_start(Stream::Stderr),
        "an unterminated chunk on stdout left the shared cursor mid-line"
    );
    f.note_written(Stream::Stderr, b"\n");
    assert!(
        f.line_start(Stream::Stdout),
        "a newline on stderr put the shared cursor back at column zero"
    );
}

/// The `fstat` triple answers "same open file", and a terminal reached through
/// two device files is one terminal with two open files: `/dev/tty` and the pts
/// slave share a cursor and share nothing else. Two ptys stand in for that pair
/// here - `/dev/tty` is not addressable from a unit test, which has no
/// controlling terminal of its own to speak of - and they make the same point
/// the same way: both are terminals, neither triple matches, and otto's two
/// handles are never on two different terminals in any invocation it sees.
///
/// Measured before the fix, with the real pair, in
/// `tests/progress_shared_terminal_test.rs`: `[t] ERR[t] finished successfully`.
#[test]
fn two_terminals_are_one_destination_whatever_fstat_says() {
    let (_a_master, a) = pty_slave();
    let (_b_master, b) = pty_slave();
    assert_ne!(
        identity(&a),
        identity(&b),
        "the fixture needs two terminals that DISAGREE on the fstat triple, or it proves nothing"
    );
    assert!(
        share_destination(a.as_raw_fd(), b.as_raw_fd()),
        "two terminals are one terminal, and one cursor"
    );
}

/// The other side of the same predicate, and the regression the per-stream line
/// state exists for: files are not terminals, so a captured run never reaches
/// the terminal test and stays decoupled.
#[test]
fn two_separate_files_are_not_one_destination() {
    let temp = tempfile::TempDir::new().unwrap();
    let out = std::fs::File::create(temp.path().join("out.log")).unwrap();
    let err = std::fs::File::create(temp.path().join("err.log")).unwrap();
    assert!(
        !share_destination(out.as_raw_fd(), err.as_raw_fd()),
        "`> out 2> err` is two cursors"
    );
}

/// And `> log 2>&1`, spelled as the two independent opens it can also be: one
/// file, so one cursor, so one line state.
#[test]
fn one_file_reached_by_two_handles_is_one_destination() {
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("both.log");
    let out = std::fs::File::create(&path).unwrap();
    let err = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    assert!(
        share_destination(out.as_raw_fd(), err.as_raw_fd()),
        "both handles on one file share its cursor"
    );
}

/// A pty master/slave pair, of which the caller wants the slave and has to keep
/// the master alive to hold it open.
fn pty_slave() -> (OwnedFd, OwnedFd) {
    let mut master_fd = 0;
    let mut slave_fd = 0;
    // Safe: both out-params are written on success and nothing else is passed
    // in.
    let rc = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            // `*mut`, not `*const`: glibc declares these two as `const` and
            // Apple's libc does not, so a null `*const` compiles on Linux and
            // fails E0308 on macOS. A null `*mut` satisfies both.
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty failed: {}", std::io::Error::last_os_error());
    // Safe: `openpty` just handed both fds over and nothing else owns them.
    unsafe { (OwnedFd::from_raw_fd(master_fd), OwnedFd::from_raw_fd(slave_fd)) }
}

/// The `(device, inode, rdev)` triple `share_destination` compares, read the
/// long way round so the test does not reach into its private helper.
fn identity(fd: &OwnedFd) -> (u64, u64, u64) {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::File::from(fd.try_clone().expect("dup the fd"))
        .metadata()
        .expect("fstat the fd");
    (meta.dev(), meta.ino(), meta.rdev())
}

/// A `tty:` child inherits BOTH of otto's streams, so the terminal whose SGR
/// state it left open is whichever of them is a terminal. Gating the whole
/// reset on stderr left `otto owner 2>log` red for the rest of the run
/// (measured, audit round 2), which is the second row here.
#[test]
fn a_reset_goes_to_whichever_handle_is_a_terminal() {
    assert_eq!(
        reset_targets(false, true, false),
        vec![Stream::Stderr],
        "a terminal stderr with stdout captured"
    );
    assert_eq!(
        reset_targets(true, false, false),
        vec![Stream::Stdout],
        "`otto owner 2>log`: the terminal the child dirtied is stdout"
    );
    assert_eq!(
        reset_targets(true, true, true),
        vec![Stream::Stderr],
        "one terminal, one cursor, one reset"
    );
    assert_eq!(
        reset_targets(false, false, false),
        Vec::<Stream>::new(),
        "a fully captured run has no attributes to reset and must gain no escape bytes"
    );
}

/// Both handback paths, because there are two and only one of them had the
/// reset: `teardown` drained held output on its own, and `abandon_run` reaches
/// it on the first SIGINT. A teardown that takes the held output first leaves
/// the handoff's later `Drop` with `None`, so `reclaim` returns before the
/// reset - which is the third assertion below.
#[test]
fn both_handback_paths_close_the_childs_sgr_state() {
    let temp = tempfile::TempDir::new().unwrap();

    let reclaimed = Facade::with_one_destination(false);
    let handoff = reclaimed.surrender("owner", temp.path().join("reclaim.log"));
    assert_eq!(reclaimed.sgr_resets(), 0, "nothing has been handed back yet");
    drop(handoff);
    assert_eq!(
        reclaimed.sgr_resets(),
        1,
        "the normal handback closes the child's SGR state"
    );

    let torn_down = Facade::with_one_destination(false);
    let handoff = torn_down.surrender("owner", temp.path().join("teardown.log"));
    torn_down.teardown();
    assert_eq!(
        torn_down.sgr_resets(),
        1,
        "teardown is a handback too: it is where a first SIGINT takes the terminal back"
    );
    drop(handoff);
    assert_eq!(
        torn_down.sgr_resets(),
        1,
        "and the handoff's Drop then finds nothing held, which is the window the reset was missing from"
    );
}

/// Teardown is reached from overlapping exit paths - a fatal error during a
/// run that is also being signalled hits two of them - so it has to be safe to
/// call more than once.
#[test]
fn teardown_is_idempotent() {
    let f = Facade::new();
    f.teardown();
    f.teardown();
    f.write(Stream::Stdout, "");
    f.teardown();
}

/// One facade for the process, or the ordering invariant guards nothing.
#[test]
fn the_process_facade_is_one_instance() {
    assert!(std::ptr::eq(facade(), facade()));
}

// ---------------------------------------------------------------------------
// The live region behind the facade (Phase 5).
// ---------------------------------------------------------------------------

/// `Quiet` installs no renderer AT ALL, which is what makes "zero renderer
/// bytes in a captured stream" a structural property rather than a renderer
/// deciding to stay quiet.
#[test]
fn quiet_installs_no_region() {
    let f = Facade::new();
    assert!(!f.set_region(ProgressMode::Quiet, &["a".to_string()]));
    assert!(
        !f.refresh(),
        "with no region there is nothing to refresh and no thread to keep"
    );
}

/// Installed once. A second arming would give the process two `MultiProgress`
/// instances drawing on one stderr, which is the doubled-renderer defect this
/// whole design exists to close, arriving from inside instead of from a nested
/// otto.
#[test]
fn a_live_region_is_installed_exactly_once() {
    let f = Facade::new();
    assert!(f.set_region(ProgressMode::Live, &["a".to_string(), "bb".to_string()]));
    assert!(!f.set_region(ProgressMode::Live, &["a".to_string()]));
    assert!(f.refresh(), "an installed region keeps the refresh thread going");
}

/// The refresh thread's stop condition is the region's absence, and teardown is
/// what creates it. One state, not a region plus a flag that can disagree with
/// it.
#[test]
fn teardown_retires_the_region_and_with_it_the_refresh_thread() {
    let f = Facade::new();
    assert!(f.set_region(ProgressMode::Live, &["a".to_string()]));
    f.teardown();
    assert!(!f.refresh(), "teardown must stop the refresh thread");
}

/// Row mutation is safe with no region installed: every scheduler call site is
/// unconditional, because whether a run has a region is the facade's business
/// and not the scheduler's.
#[test]
fn row_mutation_with_no_region_is_a_no_op() {
    let f = Facade::new();
    let clocks = TaskClocks::default();
    f.task_started("a", clocks.start("a"));
    f.task_finished("a");
    f.task_finished("never-started");
}

/// Rows are not mutated while a `tty:` task owns the terminal: the region is
/// erased for the child's whole lifetime, and touching a bar would draw on the
/// terminal otto just handed away.
#[test]
fn a_surrendered_terminal_takes_no_row_updates() {
    let temp = tempfile::TempDir::new().unwrap();
    let f = Facade::new();
    assert!(f.set_region(ProgressMode::Live, &["owner".to_string(), "a".to_string()]));
    let handoff = f.surrender("owner", temp.path().join("spill.log"));
    let clocks = TaskClocks::default();
    f.task_started("a", clocks.start("a"));
    assert!(f.refresh(), "the region is still installed while surrendered");
    drop(handoff);
    let _ = f.take_held();
}
