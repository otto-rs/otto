#![cfg(test)]

use super::*;

/// How long a stamp is separated from the reading that must see it. Every
/// assertion below is a LOWER bound on elapsed time, which is the only kind a
/// sleep can guarantee: it may overshoot, never undershoot.
const SEPARATION_MS: u64 = 20;

async fn separate() {
    tokio::time::sleep(Duration::from_millis(SEPARATION_MS)).await;
}

/// A line resets the clock, which is what a live row's "no output for Ns"
/// reads to tell a silent task from a chatty one.
#[tokio::test]
async fn a_line_advances_the_clock() {
    let clocks = TaskClocks::default();
    let clock = clocks.start("build");
    let before = clock.last_line_ms();

    separate().await;
    clock.note_line();

    assert!(
        clock.last_line_ms() >= before + SEPARATION_MS,
        "a line must stamp the clock later than it started: {} vs {before}",
        clock.last_line_ms()
    );
}

/// A beat is stamped separately from a line, so spacing between beats is
/// measured from the previous beat rather than from the task's last output.
#[tokio::test]
async fn a_beat_advances_only_the_beat_stamp() {
    let clocks = TaskClocks::default();
    let clock = clocks.start("build");
    let line_before = clock.last_line_ms();
    let beat_before = clock.last_beat_ms();

    separate().await;
    clock.note_beat();

    assert!(
        clock.last_beat_ms() >= beat_before + SEPARATION_MS,
        "the beat stamp must move"
    );
    assert_eq!(
        clock.last_line_ms(),
        line_before,
        "a beat stamp is not the task producing a line"
    );
}

/// A task that has produced nothing is silent from the moment its clock
/// started, and has been running for just as long.
#[tokio::test]
async fn silence_and_elapsed_are_measured_from_the_clocks_start() {
    let clocks = TaskClocks::default();
    let clock = clocks.start("quiet");

    separate().await;

    assert!(
        clock.idle_ms() >= SEPARATION_MS,
        "a task that produced no line has been silent since its clock started: {}",
        clock.idle_ms()
    );
    assert!(
        clock.since_beat_ms() >= SEPARATION_MS,
        "a task that was never beaten is due one: {}",
        clock.since_beat_ms()
    );
    assert!(
        clock.elapsed() >= Duration::from_millis(SEPARATION_MS),
        "elapsed is the figure a live row reports: {:?}",
        clock.elapsed()
    );
}

/// Liveness decides who is a candidate; the clock map only says how long each
/// has been silent. A `tty: true` task skips `TaskStreams` entirely
/// (`task_execution.rs`), so it never gets a clock, and that absence is what
/// keeps it out of every read - by name, with no tty predicate at read time.
#[test]
fn a_live_task_with_no_clock_is_not_a_candidate() {
    let clocks = TaskClocks::default();
    clocks.start("build");

    let live = vec!["build".to_string(), "interactive".to_string()];
    let names: Vec<String> = clocks.candidates(&live).into_iter().map(|(name, _)| name).collect();

    assert_eq!(names, vec!["build".to_string()], "a task with no clock cannot be named");
    assert!(
        clocks.get("interactive").is_none(),
        "nothing created a clock for the tty task"
    );
}

/// The mirror case: a clock may outlive its child, and a stale entry is inert
/// rather than a bug, because a candidate read only sees a task the registry
/// still calls live.
#[test]
fn a_clock_whose_task_is_not_live_is_not_a_candidate() {
    let clocks = TaskClocks::default();
    clocks.start("finished");
    clocks.start("running");

    let names: Vec<String> = clocks
        .candidates(&["running".to_string()])
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    assert_eq!(names, vec!["running".to_string()]);
    assert!(
        clocks.get("finished").is_some(),
        "the stale entry stays; it is simply never read"
    );
}

/// Several candidates come back in a stable order, so a reader with more than
/// one row to draw does not shuffle them between reads.
#[test]
fn candidates_are_ordered_by_name() {
    let clocks = TaskClocks::default();
    for task in ["tail:s2", "build", "tail:s1"] {
        clocks.start(task);
    }

    let live = vec!["tail:s2".to_string(), "build".to_string(), "tail:s1".to_string()];
    let names: Vec<String> = clocks.candidates(&live).into_iter().map(|(name, _)| name).collect();

    assert_eq!(names, vec!["build", "tail:s1", "tail:s2"]);
}
