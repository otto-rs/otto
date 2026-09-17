//! The terminal-ownership gate (design doc
//! `docs/design/2026-09-16-live-progress-renderer.md`, Phase 4).
//!
//! The gate has two halves and they fail in opposite directions, so they get a
//! test each:
//!
//! - the PREDICATE: no `tty:` task is admitted while any admitted task has not
//!   yet been seen report. Its acceptance test is the pty fixture in
//!   `tests/progress_spike_pty_test.rs`, which Phase 0 shipped `#[ignore]`d
//!   because it failed on that binary and Phase 4 un-ignored.
//! - DRAIN MODE: while a `tty:` task is deferred for ownership, ordinary tasks
//!   are not admitted either. Without it a bare predicate is a REGRESSION: the
//!   launch loop defers and continues in the same pass, ordinary tasks are
//!   ungated, and the deferred tty task goes back to the head of the queue only
//!   to be deferred again, so with a ready queue of `[tty, long-1, long-2]` the
//!   two long tasks are admitted ahead of the tty task and hold it off for
//!   their whole duration. That is this file's first test.
//!
//! Wall-clock intervals the tasks stamped themselves, not status fields: the
//! claim is about scheduling, which is not observable from inside the process.

mod common;

use common::otto_cmd;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Bound on anything this file waits for. The failure being guarded is an
/// unbounded delay, so the number only has to be finite and slack.
const STEP_TIMEOUT: Duration = Duration::from_secs(60);

/// Microsecond stamp, from perl for the reasons `tests/tty_task_test.rs`
/// records: this box's uutils `date` ignores `%3N`, and `EPOCHREALTIME` is
/// bash 5.0+ while macOS runs task bodies under bash 3.2.
const STAMP: &str = r#"$(perl -MTime::HiRes=time -e 'printf "%.0f", time()*1000000')"#;

fn write_ottofile(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).expect("write the fixture ottofile");
    path
}

/// (task, start_us, end_us) parsed out of the shared timeline file.
fn intervals(timeline: &Path) -> Vec<(String, u64, u64)> {
    let text = fs::read_to_string(timeline).unwrap_or_else(|e| panic!("no timeline at {}: {e}", timeline.display()));
    let mut starts = std::collections::HashMap::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(parts.len(), 3, "malformed timeline line {line:?}");
        let stamp: u64 = parts[2]
            .parse()
            .unwrap_or_else(|e| panic!("bad stamp in {line:?}: {e}"));
        match parts[1] {
            "start" => {
                starts.insert(parts[0].to_string(), stamp);
            }
            "end" => {
                let start = starts.remove(parts[0]).expect("end before start");
                out.push((parts[0].to_string(), start, stamp));
            }
            other => panic!("unexpected timeline verb {other:?}"),
        }
    }
    out
}

fn interval(observed: &[(String, u64, u64)], name: &str) -> (u64, u64) {
    observed
        .iter()
        .find(|(task, _, _)| task == name)
        .map(|(_, start, end)| (*start, *end))
        .unwrap_or_else(|| panic!("{name} never ran: {observed:?}"))
}

/// The drain-mode fixture.
///
/// **The ready-queue order is built out of dependencies, not out of
/// declaration order.** The run set's order is a `HashMap` iteration order:
/// measured 2026-09-17, the same four independent tasks started in two
/// different orders on two consecutive runs, so a fixture that assumed
/// declaration order would be testing the allocator.
///
/// The edges make it deterministic instead:
///
/// - `active` holds the run open for 1s, so anything ready before then is
///   deferred by the ownership predicate.
/// - `gate` ends at ~0.05s, which is when `owner` becomes ready and is deferred
///   for ownership. A deferral is restored to the HEAD of the ready queue, so
///   `owner` is at the front from then on.
/// - `slowgate` ends at ~0.2s, which is when `long-1` and `long-2` become
///   ready - behind `owner`, and with drain mode already on.
///
/// So the pass that follows `active`'s report sees exactly the ready queue the
/// design doc names: `[tty, long-1, long-2]`.
fn drain_fixture(timeline: &Path) -> String {
    let body = |name: &str, tty: bool, deps: &str, sleep: &str| {
        let tty_line = if tty { "    tty: true\n" } else { "" };
        let dep_line = if deps.is_empty() { String::new() } else { format!("    before: [{deps}]\n") };
        format!(
            r#"  {name}:
{tty_line}{dep_line}    bash: |
      echo "{name} start {STAMP}" >> {timeline}
      sleep {sleep}
      echo "{name} end {STAMP}" >> {timeline}
"#,
            timeline = timeline.display()
        )
    };
    format!(
        "otto:\n  api: 1\n\ntasks:\n{}{}{}{}{}{}",
        body("active", false, "", "1"),
        body("gate", false, "", "0.05"),
        body("slowgate", false, "", "0.2"),
        body("owner", true, "gate", "0.1"),
        body("long-1", false, "slowgate", "1.5"),
        body("long-2", false, "slowgate", "1.5"),
    )
}

/// **Phase 4 success criterion.** With one active ordinary task and a ready
/// queue of `[tty, long-1, long-2]`, the tty task goes in at the next
/// replenishment - when `active` reports - and NOT after the two long tasks
/// have finished.
///
/// The long tasks sleep 1.5s against `active`'s 1s, so the two outcomes are
/// nowhere near each other: under a bare predicate the long tasks are admitted
/// at ~0.2s and `owner` starts after they end at ~1.7s.
#[test]
fn a_deferred_tty_task_goes_in_at_the_next_replenishment_not_after_the_long_tasks() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let timeline = temp.path().join("timeline");
    let ottofile = write_ottofile(temp.path(), "drain.yml", &drain_fixture(&timeline));

    let output = otto_cmd(&home)
        .arg("-o")
        .arg(&ottofile)
        .args(["-j", "6", "active", "gate", "slowgate", "owner", "long-1", "long-2"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the fixture must run clean:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let observed = intervals(&timeline);
    assert_eq!(observed.len(), 6, "all six tasks must have run: {observed:?}");
    let (active_start, active_end) = interval(&observed, "active");
    let (owner_start, owner_end) = interval(&observed, "owner");
    let (long1_start, _) = interval(&observed, "long-1");
    let (long2_start, _) = interval(&observed, "long-2");
    let (_, slowgate_end) = interval(&observed, "slowgate");

    // Two vacuous-pass guards. Without the first, `owner` was never deferred
    // and the ordering below is an accident; without the second, the long tasks
    // were not even ready when `owner` went in, so drain mode held nothing.
    assert!(
        owner_start > active_start,
        "owner started before the ordinary task did, so nothing was ever deferred: {observed:?}"
    );
    assert!(
        slowgate_end < owner_start,
        "long-1 and long-2 were not ready before owner was admitted, so drain mode was never \
         exercised: {observed:?}"
    );

    assert!(
        owner_start >= active_end,
        "owner was admitted while an ordinary task had not reported: {observed:?}"
    );
    assert!(
        owner_start < long1_start && owner_start < long2_start,
        "the long tasks overtook the tty task the loop was draining for: {observed:?}"
    );
    assert!(
        owner_end <= long1_start && owner_end <= long2_start,
        "a long task started while the tty task still owned the terminal: {observed:?}"
    );
}

/// A `tty:` task that records otto's pid and then holds the terminal until it
/// is signalled. `$PPID` inside the body IS otto: otto spawns the interpreter,
/// and a `tty:` task is not `setsid`-ed away from it.
const CANCEL_FIXTURE: &str = r#"
otto:
  api: 1

tasks:
  owner:
    tty: true
    bash: |
      echo $PPID > "${MARKERS}/otto-pid"
      touch "${MARKERS}/started"
      sleep 600
"#;

fn wait_for(path: &Path, what: &str, child: &mut Child) {
    let deadline = Instant::now() + STEP_TIMEOUT;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    panic!("timed out waiting for {what}");
}

/// Cancellation, with the terminal surrendered to a `tty:` task.
///
/// otto's writes are HELD while a `tty:` task owns the terminal, and the point
/// of holding rather than dropping them is that a message the user needs still
/// arrives. The run-cancelled notice is that message, and cancellation is the
/// path where the handoff is given back by a task body being DROPPED rather
/// than by a child being reaped - so it is also the path where a release
/// written as a line of happy-path code would never run.
#[test]
fn a_cancelled_run_still_prints_its_notice_with_the_terminal_surrendered() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let markers = temp.path().join("markers");
    fs::create_dir_all(&markers).unwrap();
    let ottofile = write_ottofile(temp.path(), "cancel.yml", CANCEL_FIXTURE);
    let ottofile_arg = ottofile.display().to_string();

    // A pty, because a `tty:` task inheriting a pipe is not the case the
    // handoff exists for.
    let mut cmd = common::pty_cmd(&[common::OTTO_BIN, "-o", &ottofile_arg, "owner"]);
    common::isolate(&mut cmd, &home);
    cmd.env("MARKERS", &markers)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("`script` should allocate a pty and run otto");

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let pipes: [Box<dyn Read + Send>; 2] = [
        Box::new(child.stdout.take().expect("stdout")),
        Box::new(child.stderr.take().expect("stderr")),
    ];
    for mut pipe in pipes {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
    }
    drop(tx);

    wait_for(
        &markers.join("started"),
        "the tty task to take the terminal",
        &mut child,
    );
    let pid = fs::read_to_string(markers.join("otto-pid")).unwrap().trim().to_string();
    // Aimed at otto alone, not at the group: the group is the test's own
    // processes, and this has to exercise otto's own teardown.
    assert!(
        Command::new("kill").arg("-TERM").arg(&pid).status().unwrap().success(),
        "could not signal otto (pid {pid})"
    );

    let deadline = Instant::now() + STEP_TIMEOUT;
    let code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status.code().unwrap_or(-1),
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            None => {
                let _ = child.kill();
                panic!("otto did not exit within {STEP_TIMEOUT:?} of the signal");
            }
        }
    };
    let mut text = String::new();
    while let Ok(bytes) = rx.recv() {
        text.push_str(&common::pty_stdout(&bytes));
    }

    assert_ne!(code, 101, "otto panicked handing the terminal back:\n{text}");
    assert!(
        text.contains("run cancelled"),
        "the run-cancelled notice never arrived, so output written while the terminal was \
         surrendered was dropped rather than held:\n{text}"
    );
}
