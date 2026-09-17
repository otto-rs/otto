#![cfg(test)]

use tempfile::TempDir;

use super::{SURRENDER_MEMORY_BYTES, Surrendered};
use crate::executor::progress::facade::{Facade, Stream};

/// Drain a surrender into two in-memory handles, so a test can read what the
/// terminal would have received without writing to the harness's terminal.
fn drained(held: Surrendered) -> (String, String) {
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    held.drain_into(&mut out, &mut err);
    (
        String::from_utf8(out).expect("stdout is utf8"),
        String::from_utf8(err).expect("stderr is utf8"),
    )
}

/// The whole contract in one line: writes made while a `tty:` task owns the
/// terminal are held, not dropped, and come back in the order they were made.
#[test]
fn held_writes_come_back_in_order_on_rearm() {
    let temp = TempDir::new().unwrap();
    let mut held = Surrendered::new("interactive", temp.path().join("otto-held.log"));

    held.push(Stream::Stdout, b"[alpha] one\n");
    held.push(Stream::Stderr, b"[alpha] warning\n");
    held.push(Stream::Stdout, b"[alpha] two\n");
    held.push(Stream::Stderr, b"[alpha] finished successfully\n");

    assert!(!held.spilled(), "four short lines must not reach the spill file");
    let (out, err) = drained(held);
    assert_eq!(out, "[alpha] one\n[alpha] two\n");
    assert_eq!(err, "[alpha] warning\n[alpha] finished successfully\n");
}

/// An empty write is not a write. `TeeWriter` and the facade's own tests hand
/// over zero bytes, and a segment per empty write would grow the hold without
/// holding anything.
#[test]
fn an_empty_write_is_not_held() {
    let temp = TempDir::new().unwrap();
    let mut held = Surrendered::new("interactive", temp.path().join("otto-held.log"));
    held.push(Stream::Stdout, b"");
    assert_eq!(held.held_bytes(), 0);
    let (out, err) = drained(held);
    assert!(out.is_empty() && err.is_empty());
}

/// The bound is the point of the whole type: past it, bytes go to the tty
/// task's own run directory and are re-streamed from there, so a replay landing
/// mid-surrender cannot cost the size of its log in memory.
#[test]
fn past_the_bound_held_output_spills_to_a_file_and_still_replays_whole() {
    let temp = TempDir::new().unwrap();
    let spill = temp.path().join("otto-held.log");
    let mut held = Surrendered::new("interactive", spill.clone());

    // One chunk under the bound, then three more: replay's shape, one 64 KiB
    // write at a time.
    let chunk = "x".repeat(SURRENDER_MEMORY_BYTES / 2);
    for _ in 0..4 {
        held.push(Stream::Stdout, chunk.as_bytes());
    }
    held.push(Stream::Stderr, b"[bulk] finished successfully\n");

    assert!(held.spilled(), "four half-bound chunks must have spilled");
    assert!(
        held.held_bytes() <= SURRENDER_MEMORY_BYTES,
        "memory grew past the bound: {} bytes",
        held.held_bytes()
    );
    assert!(spill.exists(), "the spill file must exist while surrendered");

    let (out, err) = drained(held);
    assert_eq!(out.len(), chunk.len() * 4, "every spilled byte must come back");
    assert!(out.chars().all(|c| c == 'x'));
    assert_eq!(err, "[bulk] finished successfully\n");
    assert!(!spill.exists(), "the spill file must be removed once re-streamed");
}

/// Order survives the boundary between memory and the spill file, and so does
/// the stream each record was headed for. A single concatenated spill could not
/// do either.
#[test]
fn the_spill_preserves_stream_and_order_across_the_boundary() {
    let temp = TempDir::new().unwrap();
    let mut held = Surrendered::new("interactive", temp.path().join("otto-held.log"));

    held.push(Stream::Stdout, b"before-out\n");
    held.push(Stream::Stderr, b"before-err\n");
    held.push(Stream::Stdout, "y".repeat(SURRENDER_MEMORY_BYTES).as_bytes());
    held.push(Stream::Stderr, b"after-err\n");
    held.push(Stream::Stdout, b"after-out\n");

    let (out, err) = drained(held);
    assert!(
        out.starts_with("before-out\ny"),
        "stdout lost its ordering: {}",
        &out[..24]
    );
    assert!(out.ends_with("yafter-out\n"), "a post-spill stdout write went missing");
    assert_eq!(err, "before-err\nafter-err\n");
}

/// A spill that cannot be opened must not lose output. The bytes stay in
/// memory - the lesser failure - and re-arm says so on stderr rather than
/// swallowing it.
#[test]
fn a_spill_that_cannot_be_opened_keeps_the_output_and_reports_itself() {
    let temp = TempDir::new().unwrap();
    // A directory where the spill file should be: `File::create` cannot open it.
    let spill = temp.path().join("otto-held.log");
    std::fs::create_dir(&spill).unwrap();

    let mut held = Surrendered::new("interactive", spill);
    held.push(Stream::Stdout, "z".repeat(SURRENDER_MEMORY_BYTES + 1).as_bytes());
    assert!(!held.spilled(), "nothing can have reached a file that would not open");

    let (out, err) = drained(held);
    assert_eq!(out.len(), SURRENDER_MEMORY_BYTES + 1, "held output was dropped");
    assert!(
        err.contains("could not be spilled") && err.contains("interactive"),
        "a failed spill must name itself and the task: {err}"
    );
}

/// The facade routes writes to the hold while the terminal is surrendered, and
/// routes them back to the terminal once it is not.
///
/// Asserted through `take_held` rather than by letting the guard flush, so the
/// test never writes to the harness's own terminal.
#[test]
fn the_facade_holds_writes_while_the_terminal_is_surrendered() {
    let temp = TempDir::new().unwrap();
    let f = Facade::new();

    let handoff = f.surrender("interactive", temp.path().join("otto-held.log"));
    f.write(Stream::Stdout, "[alpha] status\n");
    f.block(|streams| {
        streams
            .out
            .write_all(b"[alpha] block\n")
            .expect("held writes never fail");
        streams
            .err
            .write_all(b"[alpha] block-err\n")
            .expect("held writes never fail");
    });

    let held = f.take_held().expect("the terminal is surrendered");
    let (out, err) = drained(held);
    assert_eq!(out, "[alpha] status\n[alpha] block\n");
    assert_eq!(err, "[alpha] block-err\n");

    // Nothing is held any more, so the guard's own release has nothing to write
    // and the harness's terminal stays clean.
    drop(handoff);
    assert!(f.take_held().is_none(), "the terminal was not handed back");
}

/// A second surrender while one is live cannot exist under the admission gate;
/// if it ever does, it must not hand the terminal back on behalf of the task
/// that actually holds it.
#[test]
fn a_second_surrender_does_not_take_or_release_the_terminal() {
    let temp = TempDir::new().unwrap();
    let f = Facade::new();

    let first = f.surrender("interactive", temp.path().join("first.log"));
    {
        let _second = f.surrender("other", temp.path().join("second.log"));
        f.write(Stream::Stdout, "");
    }
    assert!(
        f.take_held().is_some(),
        "the second guard's drop released a terminal it never took"
    );
    drop(first);
}

/// A run that exits while a `tty:` task still owns the terminal has held output
/// nobody else will flush: the body that would hand ownership back is being
/// dropped. Teardown is the last writer and has to be the one that drains it.
#[test]
fn teardown_hands_the_terminal_back_rather_than_leaving_output_held() {
    let temp = TempDir::new().unwrap();
    let f = Facade::new();

    let handoff = f.surrender("interactive", temp.path().join("otto-held.log"));
    // Zero bytes: this asserts where ownership ends up, and a test must not
    // deface the harness's output to do it.
    f.write(Stream::Stderr, "");
    f.teardown();

    assert!(
        f.take_held().is_none(),
        "teardown left the terminal surrendered, so held output would die with the process"
    );
    drop(handoff);
}
