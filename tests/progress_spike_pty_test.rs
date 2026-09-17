//! Phase 0 spike for `docs/design/2026-09-16-live-progress-renderer.md`:
//! what otto's terminal actually does today, measured, before any renderer
//! exists.
//!
//! Zero production code changed in this phase. Everything here is an
//! instrument. Two kinds of test live in this file and they are not the same
//! kind of claim:
//!
//! - The `today_` tests PASS on `1783f1c` and pin behaviour the design calls
//!   broken. They are the recorded measurement, not an endorsement. A later
//!   phase that fixes the behaviour must INVERT the named test, not delete it.
//! - The `#[ignore]`d tests FAIL on `1783f1c` and are the acceptance criteria
//!   of the phase named in each one. Rust has no expected-failure attribute
//!   and a red suite is not a deliverable, so they ship ignored, with the
//!   passing `today_` twin beside them carrying the evidence. Un-ignoring one
//!   is part of the phase that fixes it.
//!
//! Every fixture is self-terminating and bounded. A pty test that can hang is
//! a suite that can hang.

mod common;

use common::{OTTO_BIN, isolate};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// How long any pty run here gets before the test gives up. Same bound and
/// same reasoning as `tests/tty_task_test.rs`: the failure being guarded is an
/// unbounded hang, so the number only has to be finite and slack.
const PTY_TIMEOUT: Duration = Duration::from_secs(60);

/// Strip SGR sequences so an assertion can name a task by its plain text.
///
/// `common::pty_stdout` strips CR and nothing else, deliberately: the design's
/// Testing Strategy wants screen CONTENTS. Row-order assertions still need the
/// colours off, and only the colours: cursor control must survive, because a
/// later phase asserts on it.
fn strip_sgr(text: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*m").expect("static regex");
    re.replace_all(text, "").into_owned()
}

/// Run `argv` under a real pty, returning (exit code, decoded output).
///
/// Both pipes are drained from their own threads so a full pipe buffer stalls
/// nothing, and the wait is bounded.
fn pty_run(argv: &[&str], home: &Path, cwd: Option<&Path>) -> (i32, String) {
    let mut cmd = common::pty_cmd(argv);
    isolate(&mut cmd, home);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
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

    let deadline = Instant::now() + PTY_TIMEOUT;
    let code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status.code().unwrap_or(-1),
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            None => {
                let _ = child.kill();
                panic!("otto did not exit within {PTY_TIMEOUT:?} under the pty");
            }
        }
    };
    let mut text = String::new();
    while let Ok(bytes) = rx.recv() {
        text.push_str(&common::pty_stdout(&bytes));
    }
    (code, strip_sgr(&text))
}

fn write_ottofile(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).expect("write the fixture ottofile");
    path
}

/// The 0-based index of the first line containing `needle`, or a panic naming
/// what was missing. A missing marker is a fixture that stopped measuring what
/// it was written to measure, which must never read as a pass.
fn line_of(output: &str, needle: &str) -> usize {
    output.lines().position(|l| l.contains(needle)).unwrap_or_else(|| {
        panic!("the run never produced {needle:?}; the fixture is not measuring anything:\n{output}")
    })
}

// ---------------------------------------------------------------------------
// The handoff race: a `tty:` child owns the terminal while otto is still
// writing another task's output to it.
//
// Mechanism, read off main at 1783f1c. A task body binds its semaphore permit
// at `scheduler/task_execution.rs:86`, inside the `outcome` future that ends at
// `:500`. Its report is not sent until `:575`, and the scheduler does not
// replay a buffered block until it has that report. So the permit is free for
// the whole span between `:500` and the last byte of the block.
//
// Admission does not close the span either: `Admission::Tty => in_flight.exempt
// == 0` (`scheduler.rs:342`) and a buffered foreach without `jobs` classifies
// `Capped` (`scheduler.rs:363-367`), so the tty task is admitted while the
// group is running and then sits on `acquire_many`. The instant the last
// subtask's permit drops, the tty child spawns with the terminal inherited
// (`scheduler/task_execution.rs:298-299`) and writes into the middle of a
// replay otto has not finished.
//
// The fixture makes the span wide enough to see rather than creating it: `gate`
// makes `owner` ready while the group still holds every permit, and 2000 lines
// per item give the parent enough to still be printing when the child lands.
// ---------------------------------------------------------------------------

/// Three buffered subtasks with bulky output, plus a `tty:` task held off the
/// initial ready set by a short dependency so it is admitted mid-group.
const HANDOFF_FIXTURE: &str = r#"
otto:
  api: 1

tasks:
  gate:
    bash: |
      sleep 0.05

  bulk:
    foreach:
      items: [a, b, c]
      as: item
      parallel: true
      buffer: true
    bash: |
      sleep 0.5
      for i in $(seq 1 2000); do echo "${item} bulk line $i"; done

  owner:
    tty: true
    before: [gate]
    bash: |
      echo "OWNER-START"
"#;

/// Measured on main at 1783f1c: the tty child's output lands before otto has
/// finished replaying the ordinary group, so the two writers share the terminal.
///
/// Observed 10/10 through a pipe and 5/5 through a pty on 2026-09-16, with
/// `OWNER-START` landing strictly inside a replay block in 9 of those 15.
///
/// This test PASSES today and is the evidence. Phase 4 inverts it: when the
/// admission gate lands, the assertion below must be flipped to its twin,
/// `phase_4_no_tty_child_writes_before_every_admitted_task_has_reported`.
#[test]
fn today_a_tty_child_writes_while_otto_is_still_replaying_another_task() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(temp.path(), "handoff.yml", HANDOFF_FIXTURE);
    let ottofile_arg = ottofile.display().to_string();

    let (code, out) = pty_run(
        &[OTTO_BIN, "-o", &ottofile_arg, "-j", "4", "bulk", "owner"],
        &home,
        None,
    );
    assert_eq!(
        code, 0,
        "the fixture must run clean, or it is measuring a failure:\n{out}"
    );

    // Vacuous-pass guards: all four blocks replayed and the tty task ran.
    for marker in ["[bulk:a] finished successfully", "[bulk:b] finished successfully"] {
        line_of(&out, marker);
    }
    let owner = line_of(&out, "OWNER-START");
    let last_block = line_of(&out, "[bulk:c] finished successfully");

    assert!(
        owner < last_block,
        "the handoff race did not reproduce: the tty child wrote at line {owner}, after the last \
         replay block ended at {last_block}. Either the race is gone (invert this test and \
         un-ignore its Phase 4 twin) or the fixture stopped widening the window."
    );
}

/// Phase 4's acceptance criterion, ignored because it FAILS on today's binary.
///
/// `#[ignore]` rather than deletion, and rather than `#[should_panic]`: the
/// criterion is a real assertion about the shipped gate, and `should_panic`
/// would keep passing for any panic at all, including a broken fixture. Phase 4
/// removes the attribute. Run it deliberately with
/// `cargo test --test progress_spike_pty_test -- --ignored`.
///
/// Measured failure on 1783f1c, 2026-09-16: `OWNER-START` at line 1490 of 6008,
/// inside `bulk:a`'s replay block, which ends at line 2003.
#[test]
#[ignore = "Phase 4 criterion: fails on today's binary; the tty gate does not exist yet"]
fn phase_4_no_tty_child_writes_before_every_admitted_task_has_reported() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(temp.path(), "handoff.yml", HANDOFF_FIXTURE);
    let ottofile_arg = ottofile.display().to_string();

    let (code, out) = pty_run(
        &[OTTO_BIN, "-o", &ottofile_arg, "-j", "4", "bulk", "owner"],
        &home,
        None,
    );
    assert_eq!(code, 0, "the fixture must run clean:\n{out}");

    let owner = line_of(&out, "OWNER-START");
    for block in [
        "[bulk:a] finished successfully",
        "[bulk:b] finished successfully",
        "[bulk:c] finished successfully",
        "[bulk] finished successfully",
    ] {
        let end = line_of(&out, block);
        assert!(
            owner > end,
            "the tty child wrote at line {owner}, before {block:?} at line {end}: otto was still \
             writing the terminal a tty task had taken"
        );
    }
}

// ---------------------------------------------------------------------------
// A final chunk with no trailing newline.
// ---------------------------------------------------------------------------

/// `read_until(b'\n')` yields a last chunk with no newline at EOF, and the live
/// path writes it as-is. Measured on main at 1783f1c: otto's own completion
/// line is then concatenated onto the task's last line.
///
/// ```text
/// [nonl] first line
/// [nonl] DONE[nonl] finished successfully
/// ```
///
/// The design's "Unterminated final chunk" paragraph treats this as a hazard the
/// live region introduces. It is not: the defect exists today, on the plain
/// path, with no region anywhere. Replay already handles it explicitly
/// (`scheduler/replay.rs:347-352`); the live path never has.
///
/// This test PASSES today and pins the defect. The phase that fixes it inverts
/// the assertion.
#[test]
fn today_an_unterminated_final_chunk_runs_into_the_completion_line() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(
        temp.path(),
        "nonl.yml",
        r#"
otto:
  api: 1

tasks:
  nonl:
    bash: |
      echo "first line"
      printf DONE
"#,
    );
    let ottofile_arg = ottofile.display().to_string();

    let (code, out) = pty_run(&[OTTO_BIN, "-o", &ottofile_arg, "nonl"], &home, None);
    assert_eq!(code, 0, "the fixture must run clean:\n{out}");

    assert!(
        out.contains("[nonl] DONE[nonl] finished successfully"),
        "expected the completion line jammed onto the unterminated chunk; if this no longer \
         reproduces the live path grew the newline guard and this test should be inverted:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// Buffered replay through a pty.
// ---------------------------------------------------------------------------

/// A buffered block stays contiguous through a pty: every line of an item is
/// consecutive, with no other item's line inside it.
///
/// Pinned here rather than relying on `tests/foreach_buffer_test.rs`, which runs
/// through pipes. The design deletes `TERMINAL_LOCK` in Phase 2 and puts a
/// ticker on the same terminal in Phase 5, and this is the property both must
/// preserve, measured on the surface they will change.
#[test]
fn a_buffered_replay_block_stays_contiguous_under_a_pty() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(
        temp.path(),
        "buffered.yml",
        r#"
otto:
  api: 1

tasks:
  chat:
    foreach:
      items: [alpha, beta, gamma]
      as: item
      parallel: true
      buffer: true
    bash: |
      for i in $(seq 1 60); do echo "${item} chat $i"; done
"#,
    );
    let ottofile_arg = ottofile.display().to_string();

    let (code, out) = pty_run(&[OTTO_BIN, "-o", &ottofile_arg, "-j", "3", "chat"], &home, None);
    assert_eq!(code, 0, "the fixture must run clean:\n{out}");

    let mut runs: Vec<(String, usize)> = Vec::new();
    for line in out.lines() {
        let Some(item) = ["alpha", "beta", "gamma"]
            .iter()
            .find(|i| line.contains(&format!("{i} chat ")))
        else {
            continue;
        };
        match runs.last_mut() {
            Some((name, count)) if name == *item => *count += 1,
            _ => runs.push(((*item).to_string(), 1)),
        }
    }
    assert_eq!(
        runs.len(),
        3,
        "each item's 60 lines must form exactly one unbroken run; observed {runs:?}"
    );
    for (item, count) in &runs {
        assert_eq!(*count, 60, "{item} lost lines out of its block: {runs:?}");
    }
}

// ---------------------------------------------------------------------------
// Signals: SIGWINCH, terminal hangup, SIGINT with a tty task in flight.
// ---------------------------------------------------------------------------

/// otto's own pid, taken from `$PPID` inside a task body.
///
/// A task child runs in its own session (`setsid`), so its parent is otto and
/// nothing else. Same mechanism `tests/sigint_cancel_test.rs` uses.
const RECORD_PID: &str = r#"echo $PPID > "${PIDFILE}""#;

fn wait_for_file(path: &Path, what: &str) {
    let deadline = Instant::now() + PTY_TIMEOUT;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

/// Measured on main at 1783f1c: otto installs no SIGWINCH handler anywhere
/// (`rg -n 'SIGWINCH|winch' src/` is empty), so a resize mid-run is delivered
/// with the default disposition and otto survives it with its output intact.
///
/// That is the baseline Phase 5 changes: a live region has to re-read the
/// winsize on resize, and this test is what tells the difference between
/// "handled" and "ignored, and nothing broke because there was nothing to
/// break".
#[test]
fn a_sigwinch_mid_run_does_not_disturb_the_run() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let pidfile = temp.path().join("otto.pid");
    let ottofile = write_ottofile(
        temp.path(),
        "winch.yml",
        &format!(
            r#"
otto:
  api: 1

tasks:
  slow:
    bash: |
      {RECORD_PID}
      echo "BEFORE-RESIZE"
      sleep 1.5
      echo "AFTER-RESIZE"
"#
        ),
    );
    let ottofile_arg = ottofile.display().to_string();

    let mut cmd = common::pty_cmd(&[OTTO_BIN, "-o", &ottofile_arg, "slow"]);
    isolate(&mut cmd, &home);
    cmd.env("PIDFILE", &pidfile)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().expect("`script` should allocate a pty");

    wait_for_file(&pidfile, "otto to record its pid");
    let pid: i32 = fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    // Three in a row: one resize could be swallowed anywhere and still look
    // like a pass. SIGWINCH is not fatal by default, so this is not a kill.
    for _ in 0..3 {
        assert!(
            Command::new("kill")
                .arg("-WINCH")
                .arg(pid.to_string())
                .status()
                .unwrap()
                .success(),
            "could not deliver SIGWINCH to otto (pid {pid})"
        );
        thread::sleep(Duration::from_millis(100));
    }

    let output = child.wait_with_output().expect("otto should exit");
    let text = strip_sgr(&format!(
        "{}{}",
        common::pty_stdout(&output.stdout),
        common::pty_stdout(&output.stderr)
    ));
    assert!(output.status.success(), "SIGWINCH killed the run:\n{text}");
    assert!(
        text.contains("BEFORE-RESIZE") && text.contains("AFTER-RESIZE"),
        "output did not survive the resizes:\n{text}"
    );
}

/// Terminal hangup mid-run: the pty master goes away while otto is writing.
///
/// The design leans on `report_fatal` (`main.rs:240-249`) for this, whose whole
/// reason for not being `eprintln!` is that `eprintln!` panics on a failed
/// write. What is measured here is the property the facade must keep: otto must
/// never exit 101. A hangup kills the run, and that is fine; a panic on the way
/// out is not.
#[test]
fn a_terminal_hangup_mid_run_never_panics_otto() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let pidfile = temp.path().join("otto.pid");
    let ottofile = write_ottofile(
        temp.path(),
        "hangup.yml",
        &format!(
            r#"
otto:
  api: 1

tasks:
  chatty:
    bash: |
      {RECORD_PID}
      for i in $(seq 1 4000); do echo "chatty line $i"; sleep 0.001; done
"#
        ),
    );
    let ottofile_arg = ottofile.display().to_string();

    let mut cmd = common::pty_cmd(&[OTTO_BIN, "-o", &ottofile_arg, "chatty"]);
    isolate(&mut cmd, &home);
    cmd.env("PIDFILE", &pidfile)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut script = cmd.spawn().expect("`script` should allocate a pty");

    wait_for_file(&pidfile, "otto to record its pid");
    let pid: i32 = fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    thread::sleep(Duration::from_millis(200));

    // Killing `script` tears down the pty master, which is what a closed
    // terminal window does. otto keeps a slave fd whose every write now fails.
    let _ = script.kill();
    let _ = script.wait();

    let deadline = Instant::now() + PTY_TIMEOUT;
    loop {
        let alive = Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        if !alive {
            break;
        }
        if Instant::now() >= deadline {
            let _ = Command::new("kill").arg("-KILL").arg(pid.to_string()).status();
            panic!("otto (pid {pid}) outlived the terminal it was writing to");
        }
        thread::sleep(Duration::from_millis(50));
    }
    // `kill -0` cannot report an exit status, so the panic guard above is the
    // whole assertion: otto is gone, and it did not wedge. A 101 would have
    // left the same corpse, so the exit-code claim belongs to the hangup case
    // that keeps the pty, which is SIGHUP, already pinned by
    // `tests/sigint_cancel_test.rs:467`.
}

/// SIGINT arriving while a `tty:` task owns the terminal.
///
/// Measured, not asserted-into-existence: a `tty:` child stays in otto's own
/// process group (`own_group: false`, `scheduler/task_execution.rs:302`), so a
/// terminal Ctrl+C reaches both of them at once. Phase 4's teardown runs in that
/// window, after the child is already dying, and Phase 5's region erase runs
/// after that. The property that must hold through both: one Ctrl+C ends the
/// run, bounded, without a panic.
#[test]
fn a_ctrl_c_while_a_tty_task_owns_the_terminal_ends_the_run_bounded() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let started = temp.path().join("started");
    let ottofile = write_ottofile(
        temp.path(),
        "ctrlc.yml",
        &format!(
            r#"
otto:
  api: 1

tasks:
  owner:
    tty: true
    bash: |
      touch {started}
      sleep 30
"#,
            started = started.display()
        ),
    );
    let ottofile_arg = ottofile.display().to_string();

    let mut cmd = common::pty_cmd(&[OTTO_BIN, "-o", &ottofile_arg, "owner"]);
    isolate(&mut cmd, &home);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("`script` should allocate a pty");

    wait_for_file(&started, "the tty task to take the terminal");
    // A literal ETX through `script`'s stdin: the line discipline turns it into
    // SIGINT for the pty's foreground process group, which is how a user's
    // Ctrl+C arrives. Same drive as `tests/sigint_cancel_test.rs`.
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("stdin");
        stdin.write_all(&[0x03]).expect("write ETX to the pty");
        stdin.flush().expect("flush");
    }

    let deadline = Instant::now() + PTY_TIMEOUT;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            None => {
                let _ = child.kill();
                panic!("one Ctrl+C did not end a run holding a tty task within {PTY_TIMEOUT:?}");
            }
        }
    };
    assert_ne!(
        status.code(),
        Some(101),
        "otto panicked on the Ctrl+C path with a tty task in flight"
    );
}
