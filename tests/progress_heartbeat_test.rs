//! The `still running` heartbeat is gone (Phase 1 of design doc
//! `docs/design/2026-09-16-live-progress-renderer.md`). This file used to be
//! end-to-end coverage that the heartbeat fired on schedule
//! (`docs/design/2026-09-15-idle-task-heartbeat.md`, Phase 3, superseded);
//! it is now the mirror claim - that no such line is emitted, however long a
//! task stays silent and whatever `--progress-interval` is given, because the
//! ticker that used to emit it no longer exists.
//!
//! `--progress-interval` itself is untouched CLI surface (Phase 3 of the new
//! doc deprecates it); these tests only assert it no longer produces output.

mod common;

use common::{OTTO_BIN, isolate};
use std::fs;
use std::process::Stdio;

use tempfile::TempDir;

/// A task silent for longer than the shortest interval a caller could ask
/// for, so a surviving ticker would have had every chance to beat.
const OTTOFILE: &str = r#"
tasks:
  quiet:
    bash: |
      echo START
      sleep 2
  crbar:
    bash: |
      for i in {1..6}; do
        printf '\rprogress %d' "$i"
        sleep 0.3
      done
"#;

fn project() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("otto.yml"), OTTOFILE).expect("write ottofile");
    let home = dir.path().join("otto-home");
    fs::create_dir_all(&home).expect("create otto home");
    (dir, home)
}

/// Run `task` with stderr merged into stdout, at the given `--progress-interval`.
fn run_merged(interval: u64, task: &str) -> (bool, String) {
    let (dir, home) = project();
    let mut cmd = std::process::Command::new(OTTO_BIN);
    cmd.args(["--progress-interval", &interval.to_string(), task]);
    isolate(&mut cmd, &home);
    let output = cmd
        .current_dir(dir.path())
        .env_remove("OTTOFILE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run otto");

    let mut merged = String::from_utf8_lossy(&output.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), merged)
}

/// The positive case the heartbeat used to own: a task silent for the whole
/// run emits nothing periodic, at an interval short enough that a surviving
/// ticker would have beaten several times.
#[test]
fn a_silent_task_emits_no_still_running_line() {
    let (ok, out) = run_merged(1, "quiet");
    assert!(ok, "the run must succeed:\n{out}");
    assert!(
        out.contains("START"),
        "the task's own line must still be captured:\n{out}"
    );
    assert!(
        !out.contains("still running"),
        "no heartbeat may survive Phase 1's removal:\n{out}"
    );
}

/// `0` used to be the heartbeat's off switch. It remains an accepted value -
/// Phase 3 of the new design deprecates the flag rather than removing it,
/// warning on stderr instead of erroring - and still produces no heartbeat
/// line, same as every other value now.
#[test]
fn a_zero_progress_interval_is_still_accepted_and_emits_nothing() {
    let (ok, out) = run_merged(0, "quiet");
    assert!(ok, "--progress-interval 0 must still be accepted:\n{out}");
    assert!(!out.contains("still running"), "no heartbeat may appear:\n{out}");
    assert!(
        out.contains("deprecated"),
        "an explicit --progress-interval must warn (Phase 3):\n{out}"
    );
}

/// The motivating case: a `\r`-redrawn bar that never emits a newline used to
/// be the shape most likely to get heartbeated, because the idle clock never
/// saw a line to reset it. It gets no beat either, because nothing beats.
#[test]
fn a_carriage_return_bar_with_no_newline_emits_no_still_running_line() {
    let (ok, out) = run_merged(1, "crbar");
    assert!(ok, "the run must succeed:\n{out}");
    assert!(!out.contains("still running"), "no heartbeat may appear:\n{out}");
}
