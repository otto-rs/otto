#![cfg(test)]

use super::*;

/// How long a stamp is separated from the reading that must see it. Every
/// assertion below is a LOWER bound on elapsed time, which is the only kind a
/// sleep can guarantee: it may overshoot, never undershoot.
const SEPARATION_MS: u64 = 20;

async fn separate() {
    tokio::time::sleep(Duration::from_millis(SEPARATION_MS)).await;
}

/// A line resets the clock, which is what makes a chatty task silent to the
/// heartbeat.
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
        "being heartbeated is not the task producing a line"
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
        "a task that was never heartbeated is due one: {}",
        clock.since_beat_ms()
    );
    assert!(
        clock.elapsed() >= Duration::from_millis(SEPARATION_MS),
        "elapsed is the figure the line reports: {:?}",
        clock.elapsed()
    );
}

/// Liveness decides who is a candidate; the clock map only says how long each
/// has been silent. A `tty: true` task skips `TaskStreams` entirely
/// (`task_execution.rs`), so it never gets a clock, and that absence is what
/// keeps it out of every tick - by name, with no tty predicate at tick time.
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
/// rather than a bug, because a tick reads it only for a task the registry
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

/// Several candidates come back in a stable order, so a tick that has more
/// than one line to emit does not shuffle them between beats.
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

/// A tick sleeps at the interval, but never longer than a second: a task can
/// fall silent at any point between two wakes, so a coarse wake is what makes
/// a beat late.
#[test]
fn a_tick_wakes_at_the_interval_or_every_second_whichever_is_shorter() {
    assert_eq!(wake_granularity(Duration::from_millis(250)), Duration::from_millis(250));
    assert_eq!(wake_granularity(Duration::from_secs(1)), Duration::from_secs(1));
    assert_eq!(wake_granularity(Duration::from_secs(600)), MAX_WAKE);
}

/// The line has to survive being captured to a log verbatim, which is the
/// whole reason it is a line rather than a spinner.
#[test]
fn a_beat_line_carries_no_escape_and_no_carriage_return() {
    let line = beat_line("philo", Duration::from_secs(134), false, false);

    assert_eq!(line, "[philo] still running (2m14s)\n");
    assert!(!line.contains('\r'), "no carriage return: {line:?}");
    assert!(!line.contains('\u{1b}'), "no escape byte: {line:?}");
}

/// Elapsed comes from otto's own `format_duration`, so the sub-minute form is
/// one decimal place. That is a consequence of reuse and is pinned here so a
/// second duration formatter cannot quietly appear.
#[test]
fn a_beat_line_reports_elapsed_in_ottos_own_duration_format() {
    assert_eq!(
        beat_line("build", Duration::from_millis(45_000), false, false),
        "[build] still running (45.0s)\n"
    );
}

/// `--no-prefix` reaches otto's own lines too, so a run that asked for no
/// `[task]` prefix does not get one back from the heartbeat.
#[test]
fn no_prefix_drops_the_brackets() {
    assert_eq!(
        beat_line("build", Duration::from_secs(90), true, false),
        "build still running (1m30s)\n"
    );
}

/// Whether to colour is decided by asking about STDERR, the stream the line is
/// written to. `task_label` answers for stdout - `colored` derives
/// `SHOULD_COLORIZE` from `stdout().is_terminal()` - and applying that decision
/// to stderr is how colour reaches a redirected stderr in otto today. The
/// heartbeat picks between the two rather than inheriting one.
#[test]
fn the_label_form_follows_stderr_not_stdout() {
    let redirected = beat_line("build", Duration::from_secs(90), false, false);
    let terminal = beat_line("build", Duration::from_secs(90), false, true);

    assert_eq!(
        redirected,
        format!("{} still running (1m30s)\n", plain_task_label("build", false))
    );
    assert_eq!(
        terminal,
        format!("{} still running (1m30s)\n", task_label("build", false))
    );
}

/// The positive case: a live task that has produced no line for the interval.
#[test]
fn a_silent_live_task_is_due_a_beat() {
    let clocks = TaskClocks::default();
    clocks.start("quiet");

    let due = due_beats(&clocks, &["quiet".to_string()], 0);

    assert_eq!(
        due.into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
        vec!["quiet".to_string()]
    );
}

/// The negative case the whole idle clock exists for: a task that just produced
/// a line is not silent, so it is not due a beat however long it has run.
#[test]
fn a_task_that_just_produced_a_line_is_not_due_a_beat() {
    let clocks = TaskClocks::default();
    let clock = clocks.start("chatty");
    clock.note_line();

    assert!(
        due_beats(&clocks, &["chatty".to_string()], 60_000).is_empty(),
        "a line inside the interval means the task is not silent"
    );
}

/// Spacing is measured from the previous beat, so a task silent for an hour is
/// printed once per interval rather than on every wake.
#[test]
fn a_task_beaten_inside_the_interval_is_not_due_another() {
    let clocks = TaskClocks::default();
    let clock = clocks.start("quiet");
    clock.note_beat();

    assert!(
        due_beats(&clocks, &["quiet".to_string()], 60_000).is_empty(),
        "a beat inside the interval means the next one is not due yet"
    );
}

/// `--progress-interval 0` is the off switch, and it costs nothing: no thread
/// is started at all, so there is nothing that could emit.
#[test]
fn a_zero_interval_starts_no_thread() {
    let heartbeat = spawn(Duration::ZERO, false, Arc::new(TaskClocks::default()), Vec::new);

    assert!(heartbeat.thread.is_none(), "zero disables the ticker outright");
}

/// The ticker really does beat: the stamp only moves in `tick`, after the line
/// is written, so an advanced beat stamp is proof the whole thread path ran.
#[test]
fn the_ticker_beats_a_silent_live_task() {
    let clocks = Arc::new(TaskClocks::default());
    let clock = clocks.start("quiet");
    let before = clock.last_beat_ms();

    let mut heartbeat = spawn(Duration::from_secs(1), false, clocks.clone(), || {
        vec!["quiet".to_string()]
    });
    thread::sleep(Duration::from_millis(1_500));
    heartbeat.stop();

    assert!(
        clock.last_beat_ms() > before,
        "a second of silence at a one-second interval must produce a beat: {} vs {before}",
        clock.last_beat_ms()
    );
}

/// Stopping is prompt, and that is why the ticker sleeps on a condvar rather
/// than polling a flag: a `sleep(granularity)` loop would hold otto's exit for
/// up to a second after the run's final status line. It is also what makes "no
/// heartbeat follows the final status line" a guarantee - `stop` joins, so a
/// tick already mid-write finishes before the run reports.
#[test]
fn stopping_is_prompt_and_the_ticker_stops_looking() {
    let wakes = Arc::new(AtomicU64::new(0));
    let counter = wakes.clone();
    let mut heartbeat = spawn(
        Duration::from_secs(1),
        false,
        Arc::new(TaskClocks::default()),
        move || {
            counter.fetch_add(1, Ordering::Relaxed);
            Vec::new()
        },
    );

    thread::sleep(Duration::from_millis(1_200));
    let stopping = Instant::now();
    heartbeat.stop();
    let stopped = stopping.elapsed();

    let after_stop = wakes.load(Ordering::Relaxed);
    assert!(
        after_stop >= 1,
        "the ticker must have woken at least once before the stop: {after_stop}"
    );
    assert!(
        stopped < Duration::from_millis(500),
        "stop must not wait out the wake granularity: {stopped:?}"
    );

    thread::sleep(Duration::from_millis(1_200));
    assert_eq!(
        wakes.load(Ordering::Relaxed),
        after_stop,
        "a stopped ticker reads liveness no further"
    );
}
