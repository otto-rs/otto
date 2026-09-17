#![cfg(test)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use super::{Facade, Stream, facade};

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
