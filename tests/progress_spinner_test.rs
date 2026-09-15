//! The spinner's two promises, checked against the real binary.
//!
//! The first is the one the feature had to earn before it could exist: a run
//! whose output is captured must be byte-identical to one without the spinner
//! compiled in. "If there's no way to show the spinner without defacing logs
//! then it's not worth the visual indication" was the bar, so it is a test and
//! not a comment.
//!
//! The second is that it appears at all, which needs a pty -- `assert_cmd`
//! gives the child pipes, and pipes are exactly the case where a spinner must
//! stay silent. `common::pty_cmd` is the same helper the CLI-surface tests use
//! for this.

mod common;

use common::{OTTO_BIN, isolate, otto_std_cmd, pty_cmd, pty_stdout};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// A task that prints, then says nothing for longer than the idle threshold,
/// then prints again: the shape of the slim-dump download that prompted this.
const QUIET_TASK: &str = r#"
otto:
  api: 1
  tasks:
    - quiet
tasks:
  quiet:
    help: goes silent for longer than the idle threshold
    bash: |
      echo "before"
      sleep 5
      echo "after"
"#;

fn fixture(dir: &Path) {
    fs::write(dir.join("otto.yml"), QUIET_TASK).expect("write ottofile");
}

/// Captured output carries no spinner. Not "few frames" or "mostly clean":
/// no escape byte and no carriage return, on either stream.
///
/// This is the whole promise. A pipe is what a log file, a CI job, a
/// `$(...)` capture and a nested otto all look like from in here.
#[test]
fn a_captured_run_contains_no_spinner_at_all() {
    let dir = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    fixture(dir.path());

    let out = otto_std_cmd(home.path())
        .current_dir(dir.path())
        .env_remove("OTTOFILE")
        .env("TERM", "xterm-256color")
        .arg("quiet")
        .output()
        .expect("run otto");

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    for (name, bytes) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
        assert!(
            !bytes.contains(&0x1b),
            "{name} carries an escape byte: {:?}",
            String::from_utf8_lossy(bytes)
        );
        assert!(
            !bytes.contains(&b'\r'),
            "{name} carries a carriage return: {:?}",
            String::from_utf8_lossy(bytes)
        );
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("before") && stdout.contains("after"), "{stdout}");
}

/// On a terminal, a run that goes quiet does spin -- and the frame names the
/// task and counts, rather than being a bare glyph.
#[test]
fn a_quiet_run_on_a_terminal_spins() {
    let dir = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    fixture(dir.path());

    let mut cmd = pty_cmd(&[OTTO_BIN, "quiet"]);
    isolate(&mut cmd, home.path());
    let out = cmd
        .current_dir(dir.path())
        .env_remove("OTTOFILE")
        .env("TERM", "xterm-256color")
        .output()
        .expect("run otto under a pty");

    let text = pty_stdout(&out.stdout);
    assert!(
        text.contains('⠋') || text.contains('⠙') || text.contains('⠹'),
        "expected a spinner frame on a terminal, got: {text:?}"
    );
    assert!(text.contains("quiet"), "the frame names the task: {text:?}");
}

/// ...and every documented way of saying "not here" is honoured, on the same
/// terminal that would otherwise spin. One case per reason, because each is a
/// separate promise to a different caller: a recorded session, a CI job, a
/// NO_COLOR user, and an explicit flag.
#[test]
fn a_terminal_run_can_still_be_told_not_to_spin() {
    for (flag, env) in [
        (Some("--no-progress"), None),
        (None, Some(("OTTO_NO_PROGRESS", "1"))),
        (None, Some(("CI", "true"))),
        (None, Some(("NO_COLOR", "1"))),
        (None, Some(("TERM", "dumb"))),
    ] {
        let dir = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        fixture(dir.path());

        let mut argv = vec![OTTO_BIN];
        if let Some(f) = flag {
            argv.push(f);
        }
        argv.push("quiet");

        let mut cmd = pty_cmd(&argv);
        isolate(&mut cmd, home.path());
        cmd.current_dir(dir.path())
            .env_remove("OTTOFILE")
            .env("TERM", "xterm-256color");
        if let Some((k, v)) = env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run otto under a pty");
        let text = pty_stdout(&out.stdout);

        let why = flag
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}={}", env.unwrap().0, env.unwrap().1));
        assert!(
            !text.contains('⠋') && !text.contains('⠙') && !text.contains('⠹'),
            "{why} must suppress the spinner, got: {text:?}"
        );
    }
}
