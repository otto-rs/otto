//! Phase 5 of `docs/design/2026-09-16-live-progress-renderer.md`: the live
//! region, and the completion line.
//!
//! Four of these are the phase's own success criteria; the fifth is the
//! Acceptance Criteria row the design records as unrunnable until a renderer
//! exists ("a nested otto run emits exactly one live region"), which is this
//! phase.
//!
//! Screen CONTENTS, not stripped text, per the design's Testing Strategy: the
//! region's whole risk is that it overwrites or erases output that was already
//! on the screen, and an assertion over escape-stripped text cannot see that.
//! [`strip_sgr`] takes the colours off so a row can be named by its plain text;
//! cursor control survives, and [`strip_csi`] is the one that takes it away,
//! for the assertions that are about where a newline is.
//!
//! Every fixture is self-terminating and bounded. A pty test that can hang is a
//! suite that can hang.

mod common;

use common::{OTTO_BIN, isolate, otto_cmd};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// How long any pty run here gets before the test gives up. Same bound and
/// reasoning as `tests/progress_spike_pty_test.rs`: the failure being guarded
/// is an unbounded hang, so the number only has to be finite and slack.
const PTY_TIMEOUT: Duration = Duration::from_secs(60);

/// Strip SGR sequences so an assertion can name a row by its plain text.
fn strip_sgr(text: &str) -> String {
    regex::Regex::new(r"\x1b\[[0-9;]*m")
        .expect("static regex")
        .replace_all(text, "")
        .into_owned()
}

/// Strip every CSI sequence, colours and cursor control alike.
fn strip_csi(text: &str) -> String {
    regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
        .expect("static regex")
        .replace_all(text, "")
        .into_owned()
}

fn write_ottofile(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).expect("write the fixture ottofile");
    path
}

/// Run `argv` under a real pty, returning (exit code, output with colours
/// stripped and cursor control intact).
fn pty_run(argv: &[&str], home: &Path) -> (i32, String) {
    let mut cmd = common::pty_cmd(argv);
    isolate(&mut cmd, home);
    common::expect_live_region(&mut cmd);
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

/// Every whole-second duration a row for `task` reported, in order.
///
/// Rows read `<name>  running <n>s`; the fixtures below all run for well under
/// a minute, so one shape covers them.
fn row_seconds(output: &str, task: &str) -> Vec<u64> {
    regex::Regex::new(&format!(r"{task}\s+running (\d+)s"))
        .expect("row regex")
        .captures_iter(output)
        .map(|c| c[1].parse().expect("digits"))
        .collect()
}

/// The whole-second duration on `task`'s completion line.
fn completion_seconds(output: &str, task: &str) -> u64 {
    let re = regex::Regex::new(&format!(r"\[{task}\] finished successfully\s+(\d+)s")).expect("completion regex");
    re.captures(output)
        .unwrap_or_else(|| panic!("no completion line with a duration for {task:?}:\n{output}"))[1]
        .parse()
        .expect("digits")
}

// ---------------------------------------------------------------------------
// Criterion 1: screen contents, a chatty task beside a silent one.
// ---------------------------------------------------------------------------

/// The fixture the first two criteria share, so the `auto` and `never` runs are
/// answering about the same run rather than about two different ones.
///
/// `silent` sleeps past the row's idle threshold on purpose: the row has to say
/// how long the silence has lasted, which is the whole complaint the design
/// exists to answer, and a 1s sleep would never reach it.
const CHATTY_AND_SILENT: &str = r#"
otto:
  api: 1

tasks:
  chatty:
    bash: |
      for i in 1 2 3 4 5 6 7 8 9 10; do
        echo "chatty line $i"
        sleep 0.2
      done
  silent:
    bash: |
      sleep 7
"#;

#[test]
fn a_chatty_task_and_a_silent_one_each_get_a_row_that_says_what_it_is_doing() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(temp.path(), "rows.yml", CHATTY_AND_SILENT);
    let ottofile_arg = ottofile.display().to_string();

    let (code, out) = pty_run(&[OTTO_BIN, "-o", &ottofile_arg, "chatty", "silent"], &home);
    assert_eq!(code, 0, "the fixture must run clean:\n{out}");

    // The chatty task's output is still on the screen, verbatim and complete.
    // The region draws below it and erases only its own rows; a region that
    // erased scrollback would take these with it.
    for i in 1..=10 {
        assert!(
            out.contains(&format!("[chatty] chatty line {i}")),
            "the region ate chatty's line {i}:\n{out}"
        );
    }

    // Both tasks got a row, and each row names its own elapsed time.
    assert!(
        !row_seconds(&out, "chatty").is_empty(),
        "no live row for the chatty task:\n{out}"
    );
    let silent_rows = row_seconds(&out, "silent");
    assert!(!silent_rows.is_empty(), "no live row for the silent task:\n{out}");
    assert!(
        silent_rows.iter().any(|&s| s >= 5),
        "the silent task's row never counted past 5s in a 7s run: {silent_rows:?}\n{out}"
    );

    // And the silent one says what it is silent about, which is the defect the
    // v2.5.3 heartbeat got wrong: a bare number naming no subject.
    assert!(
        out.contains("no output for"),
        "a task silent for 7s never reported its silence:\n{out}"
    );

    // The rule between scrollback and the rows.
    assert!(out.contains("-----"), "the region drew no separator:\n{out}");

    // Completion lines still arrive, each carrying its own duration.
    assert!(completion_seconds(&out, "chatty") >= 2, "chatty ran for ~2s:\n{out}");
    assert!(completion_seconds(&out, "silent") >= 6, "silent ran for ~7s:\n{out}");
}

// ---------------------------------------------------------------------------
// Criterion 2: zero renderer bytes under `--progress never`.
// ---------------------------------------------------------------------------

/// Run under a PTY, not a pipe, on purpose. Through a pipe `auto` would also
/// be quiet, so a piped run proves the tty gate and not the flag. On a pty the
/// flag is the only thing that can be doing the silencing.
#[test]
fn progress_never_draws_nothing_while_task_output_and_completion_lines_still_arrive() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(temp.path(), "rows.yml", CHATTY_AND_SILENT);
    let ottofile_arg = ottofile.display().to_string();

    let (code, out) = pty_run(
        &[OTTO_BIN, "-o", &ottofile_arg, "--progress", "never", "chatty", "silent"],
        &home,
    );
    assert_eq!(code, 0, "the fixture must run clean:\n{out}");

    // Zero renderer bytes. Three shapes, because "no escape codes at all" is
    // not the claim: colours are otto's and predate this design.
    assert!(
        row_seconds(&out, "chatty").is_empty() && row_seconds(&out, "silent").is_empty(),
        "a row was drawn under --progress never:\n{out}"
    );
    assert!(
        !out.contains("-----"),
        "the separator was drawn under --progress never:\n{out}"
    );
    assert!(
        !out.contains("no output for") && !out.contains("more running"),
        "region text was drawn under --progress never:\n{out}"
    );
    for cursor_op in ["\x1b[2K", "\x1b[1A", "\x1b[1B"] {
        assert!(
            !out.contains(cursor_op),
            "otto emitted the cursor operation {cursor_op:?} under --progress never:\n{out:?}"
        );
    }

    // And everything that is NOT the renderer still arrives.
    for i in 1..=10 {
        assert!(
            out.contains(&format!("[chatty] chatty line {i}")),
            "--progress never lost chatty's line {i}:\n{out}"
        );
    }
    assert!(completion_seconds(&out, "chatty") >= 2, "chatty ran for ~2s:\n{out}");
    assert!(completion_seconds(&out, "silent") >= 6, "silent ran for ~7s:\n{out}");
}

// ---------------------------------------------------------------------------
// Criterion 3: a row and a completion line about the same task agree.
// ---------------------------------------------------------------------------

/// `gate` is 6s and `waiter` runs for 2, so the dependency wait EXCEEDS the
/// tolerance. That is the design doc's own instruction and it is what makes the
/// fixture falsifiable: `task_start_times` stamps every task with one shared
/// run-start instant before dependency waits, so the old completion line for
/// `waiter` reads ~8s beside a row reading 2s. A 0.2s wait plus a 0.1s task
/// would produce a wrong answer of 0.3s, which passes a 1s tolerance.
const DEP_WAIT: &str = r#"
otto:
  api: 1

tasks:
  gate:
    bash: |
      sleep 6
  waiter:
    before: [gate]
    bash: |
      echo "waiter started"
      sleep 2
"#;

#[test]
fn a_task_that_waited_on_a_dependency_reports_the_same_duration_in_both_places() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(temp.path(), "depwait.yml", DEP_WAIT);
    let ottofile_arg = ottofile.display().to_string();

    let (code, out) = pty_run(&[OTTO_BIN, "-o", &ottofile_arg, "waiter"], &home);
    assert_eq!(code, 0, "the fixture must run clean:\n{out}");

    let rows = row_seconds(&out, "waiter");
    assert!(
        !rows.is_empty(),
        "the waiter never got a live row, so there is nothing to compare:\n{out}"
    );
    let last_row = *rows.iter().max().expect("non-empty");
    let completion = completion_seconds(&out, "waiter");

    assert!(
        completion.abs_diff(last_row) <= 1,
        "the live row said {last_row}s and the completion line said {completion}s; they must \
         agree within 1s, and a gap of ~6s means the completion line is measuring the \
         dependency wait:\n{out}"
    );
    // Both must be the task's own runtime, not the run's. Stated separately so
    // a failure says which of the two drifted.
    assert!(
        completion < 6,
        "the completion line counted the 6s dependency wait: {completion}s\n{out}"
    );
}

// ---------------------------------------------------------------------------
// Criterion 4: an unterminated final chunk, on both paths.
// ---------------------------------------------------------------------------

/// `printf DONE` leaves `read_until(b'\n')` with a final chunk and no newline.
/// Measured on `1783f1c` as `[nonl] DONE[nonl] finished successfully`.
const UNTERMINATED: &str = r#"
otto:
  api: 1

tasks:
  nonl:
    bash: |
      echo "first line"
      printf DONE
"#;

#[test]
fn a_completion_line_starts_on_its_own_line_after_an_unterminated_chunk_on_a_pty() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(temp.path(), "nonl.yml", UNTERMINATED);
    let ottofile_arg = ottofile.display().to_string();

    let (code, out) = pty_run(&[OTTO_BIN, "-o", &ottofile_arg, "--progress", "auto", "nonl"], &home);
    assert_eq!(code, 0, "the fixture must run clean:\n{out}");

    assert!(
        !out.contains("DONE[nonl] finished successfully"),
        "the completion line is jammed onto the unterminated chunk:\n{out}"
    );
    // On a pty the region's cursor control sits between the two, so the claim
    // is about the newline otto wrote, not about adjacency in the byte stream.
    let plain = strip_csi(&out);
    assert!(
        plain.contains("[nonl] DONE\n"),
        "otto did not terminate the child's last chunk before writing its own line:\n{plain:?}"
    );
}

#[test]
fn a_completion_line_starts_on_its_own_line_after_an_unterminated_chunk_through_a_pipe() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(temp.path(), "nonl.yml", UNTERMINATED);

    let output = otto_cmd(&home)
        .arg("-o")
        .arg(&ottofile)
        .arg("--progress")
        .arg("never")
        .arg("nonl")
        .output()
        .expect("otto should run");
    assert!(output.status.success(), "the fixture must run clean");
    let out = strip_sgr(&String::from_utf8_lossy(&output.stdout));

    assert!(
        !out.contains("DONE[nonl] finished successfully"),
        "the completion line is jammed onto the unterminated chunk:\n{out}"
    );
    // No region here, so the completion line is literally the start of a line.
    assert!(
        out.lines().any(|line| line.starts_with("[nonl] finished successfully")),
        "no line begins with the completion line:\n{out:?}"
    );
}

/// `printf ERR >&2` leaves the UNTERMINATED chunk on stderr while the
/// completion line goes to stdout. The two are different destinations here (two
/// pipes), so stdout's cursor is at column zero and needs no correction.
const UNTERMINATED_STDERR: &str = r#"
otto:
  api: 1

tasks:
  errnonl:
    bash: |
      printf ERR >&2
"#;

/// Measured regression: one `at_line_start` flag served both streams, so an
/// unterminated chunk on stderr marked STDOUT dirty too and the completion line
/// arrived with a leading newline that nothing on stdout had earned. Against
/// v2.5.3 the same run produced `[errnonl] finished successfully\n` with no
/// leading newline, which makes this an undisclosed change to captured stdout.
#[test]
fn an_unterminated_chunk_on_stderr_puts_no_newline_into_a_captured_stdout() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(temp.path(), "errnonl.yml", UNTERMINATED_STDERR);

    let output = otto_cmd(&home)
        .arg("-o")
        .arg(&ottofile)
        .arg("errnonl")
        .output()
        .expect("otto should run");
    assert!(output.status.success(), "the fixture must run clean");
    let out = strip_sgr(&String::from_utf8_lossy(&output.stdout));

    assert!(
        !out.starts_with('\n'),
        "captured stdout opens with a newline that no write to stdout called for: {out:?}"
    );
    assert!(
        out.starts_with("[errnonl] finished successfully"),
        "the completion line must be the first thing on stdout: {out:?}"
    );
}

/// The other direction of the same defect, and the case Phase 5's own criterion
/// did NOT hold in: the unterminated chunk is on a REDIRECTED stdout while
/// stderr is the terminal. The corrective newline used to go to stderr
/// unconditionally and mark the shared flag clean, so the stream that actually
/// needed one never got it and the captured file read
/// `[nonl] DONE[nonl] finished successfully` - byte for byte the shape the
/// design names as the defect being fixed.
#[test]
fn a_completion_line_starts_on_its_own_line_when_only_stdout_is_redirected() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let ottofile = write_ottofile(temp.path(), "nonl.yml", UNTERMINATED);
    let captured = temp.path().join("stdout.txt");

    // stdout to a file, stderr left on the pty: the two streams are different
    // destinations, which is the whole point of the fixture.
    let script = format!(
        "{} -o {} nonl > {}",
        shell_word(OTTO_BIN),
        shell_word(&ottofile.display().to_string()),
        shell_word(&captured.display().to_string())
    );
    let mut cmd = common::pty_cmd(&["sh", "-c", &script]);
    isolate(&mut cmd, &home);
    common::expect_live_region(&mut cmd);
    let output = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("`script` should allocate a pty and run otto");
    let pty_text = common::pty_stdout(&output.stdout);
    assert!(output.status.success(), "the fixture must run clean:\n{pty_text}");

    let out = strip_sgr(&fs::read_to_string(&captured).expect("the redirected stdout file"));
    assert!(
        !out.contains("DONE[nonl] finished successfully"),
        "the completion line is jammed onto the unterminated chunk in the captured file:\n{out:?}"
    );
    assert!(
        out.contains("[nonl] DONE\n[nonl] finished successfully"),
        "otto did not terminate the child's last chunk on the stream that needed it:\n{out:?}"
    );
}

/// Single-quote one word for the shell `script -c` hands its string to.
fn shell_word(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

// ---------------------------------------------------------------------------
// The Acceptance Criteria row that could not run until a renderer existed.
// ---------------------------------------------------------------------------

/// The motivating defect, from the other side. v2.5.3's heartbeat was ungated,
/// so an otto task whose body ran otto produced two beats per interval: the
/// inner heartbeated its own stderr and the outer re-prefixed it as task
/// output. The renderer is gated on stderr being a terminal, and an inner
/// otto's stderr is a pipe, so the inner draws nothing - no marker, no
/// detection, no handshake.
#[test]
fn a_nested_otto_draws_no_second_region() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let inner = write_ottofile(
        temp.path(),
        "inner.yml",
        r#"
otto:
  api: 1

tasks:
  deep:
    bash: |
      sleep 1
      echo "deep done"
"#,
    );
    let outer = write_ottofile(
        temp.path(),
        "outer.yml",
        &format!(
            r#"
otto:
  api: 1

tasks:
  nested:
    bash: |
      "{bin}" -o "{inner}" deep
"#,
            bin = OTTO_BIN,
            inner = inner.display(),
        ),
    );
    let outer_arg = outer.display().to_string();

    let (code, out) = pty_run(&[OTTO_BIN, "-o", &outer_arg, "nested"], &home);
    assert_eq!(code, 0, "the nested fixture must run clean:\n{out}");

    // The inner run happened.
    assert!(out.contains("deep done"), "the inner otto never ran:\n{out}");
    // Exactly one task drew rows, and it is the outer one. A row for `deep`
    // could only come from the inner otto's own region, re-prefixed as the
    // outer task's output, which is the doubled-renderer defect.
    assert!(
        !row_seconds(&out, "nested").is_empty(),
        "the outer run drew no region at all, so this proves nothing:\n{out}"
    );
    assert!(
        row_seconds(&out, "deep").is_empty(),
        "the inner otto drew its own region:\n{out}"
    );
    assert!(
        !out.contains("[nested] -----"),
        "the inner otto's separator arrived as the outer task's output:\n{out}"
    );
}
