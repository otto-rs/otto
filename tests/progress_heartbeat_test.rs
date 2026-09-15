//! End-to-end coverage for the `still running` heartbeat
//! (`docs/design/2026-09-15-idle-task-heartbeat.md`, Phase 3).
//!
//! One test per success criterion, and the criteria come in pairs on purpose:
//! a run that emits zero heartbeat lines proves nothing unless another run of
//! the same shape proves the heartbeat can emit at all. So the chatty and
//! `--progress-interval 0` tests below are read together with the silent one,
//! and every negative test also asserts the run really lasted long enough to
//! have emitted.
//!
//! These tests are wall-clock tests: the intervals the design doc pins are
//! measured against a real 25-second task, because the thing under test is a
//! timer. They run in parallel with the rest of their binary, so the suite
//! grows by roughly one task's duration rather than by six.

mod common;

use common::{OTTO_BIN, isolate, pty_cmd};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Seconds of silence the heartbeat is configured with in every positive test.
const INTERVAL: Duration = Duration::from_secs(5);

/// The widest a beat may land after the interval it is due at. The ticker wakes
/// once a second, so a task falling silent just after a wake waits the interval
/// plus up to that second; the rest is scheduling slack.
const SLACK: Duration = Duration::from_secs(2);

/// Allowance on the LOWER bound only, for the skew between otto stamping a
/// task's line and that line reaching this side of the pipe. otto measures
/// silence from the stamp and these tests measure it from the arrival, so a
/// gap otto computed as exactly the interval reads here as a hair under it.
/// Measurement error, not a beat arriving early, and it is the only place any
/// figure the design doc pins is loosened.
const SKEW: Duration = Duration::from_millis(50);

/// Three tasks, each 25 seconds long, differing only in what they print:
/// nothing, a line a second, and a `\r`-redrawn bar that never emits a newline.
/// The last is the `philo slim-dump` shape the feature exists for.
const OTTOFILE: &str = r#"
tasks:
  quiet:
    bash: |
      echo START
      sleep 25
  chatty:
    bash: |
      for i in {1..25}; do
        echo "line $i"
        sleep 1
      done
  crbar:
    bash: |
      for i in {1..50}; do
        printf '\rprogress %d' "$i"
        sleep 0.5
      done
"#;

/// One line of captured output, with the moment it arrived.
struct Line {
    at: Duration,
    text: String,
}

/// A project directory with the three-task ottofile, plus its own `OTTO_HOME`
/// so runs never touch the developer's real one.
fn project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("otto.yml"), OTTOFILE).expect("write ottofile");
    let home = dir.path().join("otto-home");
    fs::create_dir_all(&home).expect("create otto home");
    (dir, home)
}

/// Single-quote one argv element for a `sh -c` string.
fn quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// The shell command that runs `task` at `interval` seconds, with `suffix`
/// appended for the caller's redirection.
fn otto_sh(interval: u64, task: &str, suffix: &str) -> String {
    format!("{} --progress-interval {interval} {task} {suffix}", quote(OTTO_BIN))
}

/// Read `stream` to EOF, keeping the raw bytes and the arrival time of every
/// newline-terminated line.
///
/// Timed on this side of the pipe rather than by otto: the criterion is about
/// what a user watching the run observes, and a stamp otto wrote itself could
/// not catch a beat that was computed on time and printed late.
fn read_lines<R: Read + Send + 'static>(stream: R, start: Instant) -> JoinHandle<(Vec<u8>, Vec<Line>)> {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut raw = Vec::new();
        let mut lines = Vec::new();
        loop {
            let mut buf = Vec::new();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let at = start.elapsed();
                    raw.extend_from_slice(&buf);
                    lines.push(Line {
                        at,
                        text: String::from_utf8_lossy(&buf).trim_end().to_string(),
                    });
                }
            }
        }
        (raw, lines)
    })
}

/// Run one task with stderr merged into stdout on a single pipe.
///
/// Merged on purpose: the two streams' relative order is the thing two of the
/// criteria are about ("no heartbeat after the final status line"), and two
/// separate pipes read by two threads can only give that order to within
/// scheduling jitter. One pipe makes it byte order.
fn run_merged(interval: u64, task: &str) -> Vec<Line> {
    let (dir, home) = project();
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(otto_sh(interval, task, "2>&1"));
    isolate(&mut cmd, &home);
    let mut child = cmd
        .current_dir(dir.path())
        .env_remove("OTTOFILE")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn otto");

    let start = Instant::now();
    let reader = read_lines(child.stdout.take().expect("stdout pipe"), start);
    let status = child.wait().expect("wait for otto");
    let (_, lines) = reader.join().expect("reader thread");

    assert!(status.success(), "the run must succeed; captured:\n{}", render(&lines));
    lines
}

/// Every heartbeat line that names `task`.
fn beats<'a>(lines: &'a [Line], task: &str) -> Vec<&'a Line> {
    lines
        .iter()
        .filter(|line| line.text.contains("still running") && line.text.contains(task))
        .collect()
}

fn render(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| format!("  {:>6.2}s {}", line.at.as_secs_f64(), line.text))
        .collect::<Vec<_>>()
        .join("\n")
}

fn count(haystack: &[u8], byte: u8) -> usize {
    haystack.iter().filter(|b| **b == byte).count()
}

/// Assert `gap` sits in `[INTERVAL, INTERVAL + SLACK]`.
fn assert_spacing(gap: Duration, what: &str, lines: &[Line]) {
    assert!(
        gap + SKEW >= INTERVAL,
        "{what} must not be early: {gap:?} < {INTERVAL:?}\n{}",
        render(lines)
    );
    assert!(
        gap <= INTERVAL + SLACK,
        "{what} must not be late: {gap:?} > {:?}\n{}",
        INTERVAL + SLACK,
        render(lines)
    );
}

/// Criterion 1, the positive case everything else is read against: a task that
/// goes silent for 25 seconds at a 5-second interval is reported repeatedly,
/// first at the interval after its last line and then once per interval.
///
/// Criterion 6's second half rides along, and needs this run rather than the
/// `--progress-interval 0` one: no heartbeat may follow the final status line,
/// which is only a claim worth making about a run that emitted heartbeats.
#[test]
fn a_silent_task_is_reported_once_per_interval_and_never_after_the_status_line() {
    let lines = run_merged(5, "quiet");
    let beats = beats(&lines, "[quiet]");

    assert!(
        beats.len() >= 3,
        "a 25s silence at a 5s interval must report at least 3 times, got {}\n{}",
        beats.len(),
        render(&lines)
    );

    let last_output = lines
        .iter()
        .find(|line| line.text.contains("START"))
        .expect("the task's own line must be captured")
        .at;
    assert_spacing(beats[0].at - last_output, "the first beat", &lines);

    for pair in beats.windows(2) {
        assert_spacing(pair[1].at - pair[0].at, "the gap between beats", &lines);
    }

    let status = lines
        .iter()
        .rposition(|line| line.text.contains("finished successfully"))
        .expect("the run's final status line must be captured");
    let after: Vec<&str> = lines[status + 1..]
        .iter()
        .filter(|line| line.text.contains("still running"))
        .map(|line| line.text.as_str())
        .collect();
    assert!(
        after.is_empty(),
        "no heartbeat may follow the final status line, got {after:?}\n{}",
        render(&lines)
    );
}

/// Criterion 2: with both streams captured, nothing otto wrote defaces either
/// capture. No carriage return in either file, and no escape byte on stderr -
/// the goal Ian's "if there's no way to show the spinner without defacing logs
/// then it's not worth the visual indication" set.
#[test]
fn a_captured_run_carries_no_carriage_return_and_no_colour_on_stderr() {
    let (dir, home) = project();
    let out_path = dir.path().join("out.txt");
    let err_path = dir.path().join("err.txt");

    let status = std::process::Command::new(OTTO_BIN)
        .args(["--progress-interval", "5", "quiet"])
        .current_dir(dir.path())
        .env("OTTO_HOME", &home)
        .env_remove("OTTO_DB_PATH")
        .env_remove("OTTOFILE")
        .stdout(Stdio::from(File::create(&out_path).expect("create out.txt")))
        .stderr(Stdio::from(File::create(&err_path).expect("create err.txt")))
        .status()
        .expect("run otto");
    assert!(status.success(), "the run must succeed");

    let out = fs::read(&out_path).expect("read out.txt");
    let err = fs::read(&err_path).expect("read err.txt");
    let err_text = String::from_utf8_lossy(&err);

    assert!(
        err_text.matches("still running").count() >= 3,
        "the capture must contain the heartbeats it is being checked for: {err_text:?}"
    );
    assert_eq!(count(&out, b'\r'), 0, "no carriage return on captured stdout");
    assert_eq!(count(&err, b'\r'), 0, "no carriage return on captured stderr");
    assert_eq!(count(&err, 0x1b), 0, "no escape byte on captured stderr: {err_text:?}");
}

/// Criterion 3, the case otto's existing status lines still fail: stdout on a
/// terminal and stderr redirected. `colored` derives `SHOULD_COLORIZE` from
/// `stdout().is_terminal()`, so a status line reusing that decision writes
/// colour into the redirect. The heartbeat asks about stderr instead, so its
/// label is plain here.
#[test]
fn a_heartbeat_carries_no_colour_into_a_redirected_stderr_under_a_pty() {
    let (dir, home) = project();
    let err_path = dir.path().join("err.txt");
    let redirect = format!("2> {}", quote(err_path.to_str().expect("utf-8 path")));

    let mut cmd = pty_cmd(&["sh", "-c", &otto_sh(5, "quiet", &redirect)]);
    isolate(&mut cmd, &home);
    let output = cmd
        .current_dir(dir.path())
        .env_remove("OTTOFILE")
        .output()
        .expect("run otto under script");

    // Vacuous-pass guard: a pty terminates lines with CRLF, so no `\r` on the
    // child's stdout means `script` never allocated one and the split this test
    // is about never happened.
    assert!(
        output.stdout.contains(&b'\r'),
        "script must have allocated a pty; got {:?}",
        String::from_utf8_lossy(&output.stdout)
    );

    let err = fs::read(&err_path).expect("read err.txt");
    let err_text = String::from_utf8_lossy(&err);
    assert!(
        err_text.matches("still running").count() >= 3,
        "the redirect must contain the heartbeats it is being checked for: {err_text:?}"
    );
    assert_eq!(
        count(&err, 0x1b),
        0,
        "a redirected stderr must carry no colour even with stdout on a pty: {err_text:?}"
    );
}

/// Criterion 4: a task printing once per second is never silent for the
/// interval, so it is never reported. Only worth anything read beside the
/// silent-task test above, which proves this run's configuration could emit.
#[test]
fn a_chatty_task_is_never_reported() {
    let lines = run_merged(5, "chatty");
    let own: Vec<&Line> = lines.iter().filter(|line| line.text.contains("line ")).collect();

    assert!(
        own.len() >= 20,
        "the task must have printed across the whole run, got {} lines\n{}",
        own.len(),
        render(&lines)
    );
    assert!(
        own.last().expect("at least one line").at >= Duration::from_secs(20),
        "the run must have lasted long enough to have been reported\n{}",
        render(&lines)
    );
    assert!(
        beats(&lines, "[chatty]").is_empty(),
        "a task that prints every second is not silent\n{}",
        render(&lines)
    );
}

/// Criterion 5, the motivating case: a `\r`-redrawn bar emits no newline, so
/// the drain's `read_until(b'\n')` never yields it a line, the idle clock never
/// resets, and the task is reported. This is the shape `philo slim-dump` has
/// under otto, and the reason the design chose a line over a spinner.
#[test]
fn a_carriage_return_bar_with_no_newline_is_reported() {
    let lines = run_merged(5, "crbar");
    let beats = beats(&lines, "[crbar]");

    assert!(
        beats.len() >= 3,
        "a bar that never emits a newline must be reported at least 3 times, got {}\n{}",
        beats.len(),
        render(&lines)
    );
}

/// Criterion 6: `0` is the off switch, and it is off for the same run that the
/// positive test above reports three times.
#[test]
fn a_zero_interval_reports_nothing() {
    let lines = run_merged(0, "quiet");

    let finished = lines
        .iter()
        .find(|line| line.text.contains("finished successfully"))
        .expect("the run's final status line must be captured");
    assert!(
        finished.at >= Duration::from_secs(20),
        "the run must have lasted long enough to have been reported\n{}",
        render(&lines)
    );
    assert!(
        beats(&lines, "[quiet]").is_empty(),
        "--progress-interval 0 disables the heartbeat\n{}",
        render(&lines)
    );
}

/// `project()` writes an ottofile the binary can actually parse: a broken
/// fixture would make every negative test above pass for the wrong reason.
#[test]
fn the_fixture_ottofile_declares_the_three_tasks() {
    let (dir, home) = project();
    let output = std::process::Command::new(OTTO_BIN)
        .arg("--tasks")
        .current_dir(dir.path())
        .env("OTTO_HOME", &home)
        .env_remove("OTTO_DB_PATH")
        .env_remove("OTTOFILE")
        .output()
        .expect("run otto --tasks");

    let stdout = String::from_utf8_lossy(&output.stdout);
    for task in ["quiet", "chatty", "crbar"] {
        assert!(stdout.contains(task), "--tasks must list {task}: {stdout}");
    }
    assert!(
        Path::new(&dir.path().join("otto.yml")).exists(),
        "the fixture ottofile must be on disk"
    );
}
