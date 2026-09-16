//! otto must not write colour escape bytes into a redirected stderr, unless
//! `CLICOLOR_FORCE` says to.
//!
//! `colored` derives `SHOULD_COLORIZE` from `stdout().is_terminal()`
//! (`colored-3.1.1/src/control.rs:108`), and otto used to apply that stdout-derived
//! answer to whatever stream the text was printed on. With stdout on a terminal
//! and stderr in a file, three sites leaked into the file: the scheduler's
//! failure status line, a task's own stderr through `TeeWriter`, and a buffered
//! foreach subtask's replayed `stderr.log`.
//!
//! Every test here comes in a pair, because "no escape bytes" is also what a
//! binary with all colour deleted produces:
//!
//! - the three leak tests assert zero escapes in the redirect AND escapes still
//!   present on the pty stdout of the same run;
//! - [`stderr_on_a_pty_still_carries_colour`] runs the same shape with stderr
//!   left on the pty and requires the escapes to come back.
//!
//! The other direction has its own pair: `CLICOLOR_FORCE` is `colored`'s
//! highest-priority override, so the escapes must come back in the redirect
//! under it ([`clicolor_force_colours_a_redirected_stderr`]) and must not come
//! back under `CLICOLOR_FORCE=0` ([`a_zero_clicolor_force_leaves_a_redirected_stderr_plain`]),
//! which is the difference between `colored`'s `!= "0"` rule and a rule of
//! otto's own.
//!
//! `NO_COLOR`, `CLICOLOR` and `CLICOLOR_FORCE` are removed from every child,
//! then set back per test: `colored` honours all three, and any of them
//! exported in the developer's shell decides these runs instead of the test
//! doing it - at which point the zero-escape assertions hold against a binary
//! with the fix ripped out and prove nothing. `tests/common`'s `isolate` does
//! not touch them, by design; it isolates `OTTO_HOME`, not the colour
//! environment.

mod common;

use common::{OTTO_BIN, isolate, pty_cmd};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::TempDir;

/// Four shapes: a failing task that also prints to stdout (the status-line
/// leak, with its own pty guards), a failing task that prints nothing at all
/// (so every escape byte in its run must be the stderr status line's), a task
/// that writes to its own stderr, and a buffered foreach whose subtasks do.
const OTTOFILE: &str = r#"
otto:
  api: 1

tasks:
  boom:
    bash: |
      echo "boom wrote to stdout"
      exit 3
  quiet-boom:
    bash: |
      exit 3
  noisy:
    bash: |
      echo "noisy wrote to stderr" 1>&2
  fanout:
    foreach:
      items: [alpha, beta]
      as: item
      parallel: true
      buffer: true
    bash: |
      echo "${item} wrote to stderr" 1>&2
"#;

/// The ESC byte, counted by value. `tr -cd '\x1b'` cannot do this: GNU `tr` has
/// no `\x` escape and silently counts the literal characters `x`, `1` and `b`.
const ESC: u8 = 0x1b;

fn count(bytes: &[u8], byte: u8) -> usize {
    bytes.iter().filter(|b| **b == byte).count()
}

/// A project directory with the fixture ottofile, plus its own `OTTO_HOME` so
/// runs never touch the developer's real one.
fn project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("otto.yml"), OTTOFILE).expect("write ottofile");
    let home = dir.path().join("otto-home");
    fs::create_dir_all(&home).expect("create otto home");
    (dir, home)
}

/// Single-quote one argv element for the `sh -c` string `script` runs.
fn quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// Run `task` under a real pty, with `suffix` appended to the command for the
/// caller's redirection. The returned `Output`'s stdout is the pty's, so it is
/// what a user watching the run would have seen.
fn run(dir: &Path, home: &Path, task: &str, suffix: &str, colour_env: &[(&str, &str)]) -> Output {
    let command = format!("{} {task} {suffix}", quote(OTTO_BIN));
    let mut cmd = pty_cmd(&["sh", "-c", &command]);
    isolate(&mut cmd, home);
    cmd.current_dir(dir)
        .env_remove("OTTOFILE")
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE");
    for (name, value) in colour_env {
        cmd.env(name, value);
    }
    cmd.output().expect("run otto under script")
}

/// Run `task` with stdout on the pty and stderr in a file, and hand back both.
fn split_streams(dir: &Path, home: &Path, task: &str, colour_env: &[(&str, &str)]) -> (Vec<u8>, Vec<u8>) {
    let err_path = dir.join(format!("{task}-err.txt"));
    let suffix = format!("2> {}", quote(err_path.to_str().expect("utf-8 path")));
    let output = run(dir, home, task, &suffix, colour_env);
    let err = fs::read(&err_path).expect("read the stderr redirect");
    (output.stdout, err)
}

/// Assert the run really had a pty carrying colour on stdout. Without this the
/// stderr assertions below would also pass for a `script` that allocated no
/// terminal, which is the one environment where otto is right to print plain.
fn assert_pty_stdout_is_coloured(stdout: &[u8]) {
    assert!(
        stdout.contains(&b'\r'),
        "script must have allocated a pty (a pty ends lines with CRLF); got {:?}",
        String::from_utf8_lossy(stdout)
    );
    assert!(
        count(stdout, ESC) > 0,
        "stdout on a terminal must still be coloured, or the absence of colour on stderr means nothing; got {:?}",
        String::from_utf8_lossy(stdout)
    );
}

/// Leak site 1: the scheduler's failure status line, which goes to stderr while
/// sharing its label with the success line that goes to stdout.
#[test]
fn a_failure_status_line_carries_no_colour_into_a_redirected_stderr() {
    let (dir, home) = project();
    let (stdout, err) = split_streams(dir.path(), &home, "boom", &[]);
    let err_text = String::from_utf8_lossy(&err);

    assert_pty_stdout_is_coloured(&stdout);
    assert!(
        err_text.contains("[boom] failed"),
        "the redirect must contain the plain status line it is being checked for: {err_text:?}"
    );
    assert_eq!(
        count(&err, ESC),
        0,
        "a failure status line must carry no colour into a redirected stderr: {err_text:?}"
    );
}

/// Leak site 2: a task's own stderr, prefixed by `TeeWriter` with the same
/// label the live stdout leg uses.
#[test]
fn a_tasks_own_stderr_carries_no_colour_into_a_redirected_stderr() {
    let (dir, home) = project();
    let (stdout, err) = split_streams(dir.path(), &home, "noisy", &[]);
    let err_text = String::from_utf8_lossy(&err);

    assert_pty_stdout_is_coloured(&stdout);
    assert!(
        err_text.contains("[noisy] noisy wrote to stderr"),
        "the redirect must contain the plain prefixed line it is being checked for: {err_text:?}"
    );
    assert_eq!(
        count(&err, ESC),
        0,
        "a task's own stderr must carry no colour into a redirected stderr: {err_text:?}"
    );
}

/// Leak site 3: a buffered foreach subtask reaches the terminal only through
/// ordered replay, which streams `stderr.log` to stderr on its own call and so
/// needed its own answer about the stream.
#[test]
fn a_replayed_buffered_subtask_stderr_carries_no_colour_into_a_redirected_stderr() {
    let (dir, home) = project();
    let (stdout, err) = split_streams(dir.path(), &home, "fanout", &[]);
    let err_text = String::from_utf8_lossy(&err);

    assert_pty_stdout_is_coloured(&stdout);
    for subtask in ["alpha", "beta"] {
        assert!(
            err_text.contains(&format!("[fanout:{subtask}] {subtask} wrote to stderr")),
            "the redirect must contain the plain replayed line for {subtask}: {err_text:?}"
        );
    }
    assert_eq!(
        count(&err, ESC),
        0,
        "a replayed subtask's stderr must carry no colour into a redirected stderr: {err_text:?}"
    );
}

/// The other direction, and the reason the fix asks about stderr rather than
/// suppressing colour on it: with stderr left on the pty, the failure status
/// line is coloured again.
///
/// `quiet-boom` prints nothing to stdout, so every escape byte in this run
/// belongs to the stderr status line. [`a_stdout_only_failing_task_colours_nothing_on_stdout`]
/// pins that premise.
#[test]
fn stderr_on_a_pty_still_carries_colour() {
    let (dir, home) = project();
    let output = run(dir.path(), &home, "quiet-boom", "2>&1", &[]);
    let text = String::from_utf8_lossy(&output.stdout);

    assert!(
        text.contains("failed"),
        "the run must have reached its failure status line: {text:?}"
    );
    assert!(
        count(&output.stdout, ESC) >= 6,
        "a status line on a terminal stderr keeps its colour (3 coloured spans, 6 escapes): {text:?}"
    );
}

/// The premise [`stderr_on_a_pty_still_carries_colour`] rests on: `quiet-boom`
/// writes nothing to stdout, so its pty run has no stdout colour to be
/// mistaken for stderr's.
#[test]
fn a_stdout_only_failing_task_colours_nothing_on_stdout() {
    let (dir, home) = project();
    let (stdout, err) = split_streams(dir.path(), &home, "quiet-boom", &[]);

    assert!(
        String::from_utf8_lossy(&err).contains("[quiet-boom] failed"),
        "the run must have reached its failure status line: {:?}",
        String::from_utf8_lossy(&err)
    );
    assert_eq!(
        count(&stdout, ESC),
        0,
        "quiet-boom writes nothing to stdout, so nothing there can be coloured; got {:?}",
        String::from_utf8_lossy(&stdout)
    );
}

/// `CLICOLOR_FORCE` is `colored`'s highest-priority override (`from_env`,
/// `colored-3.1.1/src/control.rs:102`), and the knob a user reaches for to pipe
/// colour into a file. The first fix for the leak above ANDed a terminal check
/// in front of `colored`'s decision on stderr, which left this knob dead there.
#[test]
fn clicolor_force_colours_a_redirected_stderr() {
    let (dir, home) = project();
    let (stdout, err) = split_streams(dir.path(), &home, "boom", &[("CLICOLOR_FORCE", "1")]);
    let err_text = String::from_utf8_lossy(&err);

    assert_pty_stdout_is_coloured(&stdout);
    assert!(
        err_text.contains("failed"),
        "the run must have reached its failure status line: {err_text:?}"
    );
    assert!(
        count(&err, ESC) >= 6,
        "under CLICOLOR_FORCE the status line keeps its colour in the redirect \
         (3 coloured spans, 6 escapes): {err_text:?}"
    );
}

/// The `!= "0"` half of `colored`'s `normalize_env`
/// (`colored-3.1.1/src/control.rs:144`): the variable being *set* is not the
/// rule, so `CLICOLOR_FORCE=0` forces nothing and the redirect stays plain.
/// Without this, "is set" would pass [`clicolor_force_colours_a_redirected_stderr`]
/// while disagreeing with `colored` about `CLICOLOR_FORCE=0` on stdout.
#[test]
fn a_zero_clicolor_force_leaves_a_redirected_stderr_plain() {
    let (dir, home) = project();
    let (stdout, err) = split_streams(dir.path(), &home, "boom", &[("CLICOLOR_FORCE", "0")]);
    let err_text = String::from_utf8_lossy(&err);

    assert_pty_stdout_is_coloured(&stdout);
    assert!(
        err_text.contains("[boom] failed"),
        "the redirect must contain the plain status line it is being checked for: {err_text:?}"
    );
    assert_eq!(
        count(&err, ESC),
        0,
        "CLICOLOR_FORCE=0 forces nothing, so the redirect stays plain: {err_text:?}"
    );
}

/// Force outranks `NO_COLOR`, in that order, per `resolve_clicolor_force`
/// (`colored-3.1.1/src/control.rs:148`). otto's stderr predicate must not
/// reorder them: with both set, stdout is coloured, so a plain stderr would be
/// otto disagreeing with itself across its two streams.
#[test]
fn clicolor_force_beats_no_color_on_a_redirected_stderr() {
    let (dir, home) = project();
    let env = [("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")];
    let (stdout, err) = split_streams(dir.path(), &home, "boom", &env);
    let err_text = String::from_utf8_lossy(&err);

    assert_pty_stdout_is_coloured(&stdout);
    assert!(
        count(&err, ESC) >= 6,
        "CLICOLOR_FORCE outranks NO_COLOR, so the status line keeps its colour: {err_text:?}"
    );
}

/// The premise the three tests above rest on: `NO_COLOR` alone still strips
/// otto's stdout. If it did not, the pty-stdout guard in
/// [`clicolor_force_beats_no_color_on_a_redirected_stderr`] would be passing
/// for a reason unrelated to the force.
#[test]
fn no_color_alone_strips_stdout_too() {
    let (dir, home) = project();
    let (stdout, err) = split_streams(dir.path(), &home, "boom", &[("NO_COLOR", "1")]);

    assert!(
        stdout.contains(&b'\r'),
        "script must have allocated a pty (a pty ends lines with CRLF); got {:?}",
        String::from_utf8_lossy(&stdout)
    );
    assert_eq!(
        count(&stdout, ESC),
        0,
        "NO_COLOR must strip stdout colour: {:?}",
        String::from_utf8_lossy(&stdout)
    );
    assert_eq!(
        count(&err, ESC),
        0,
        "NO_COLOR must strip the redirect too: {:?}",
        String::from_utf8_lossy(&err)
    );
}

/// `project()` writes an ottofile the binary can actually parse: a broken
/// fixture would make every zero-escape assertion above pass for the wrong
/// reason.
#[test]
fn the_fixture_ottofile_declares_its_tasks() {
    let (dir, home) = project();
    let output = common::otto_cmd(&home)
        .arg("--tasks")
        .current_dir(dir.path())
        .env_remove("OTTOFILE")
        .output()
        .expect("run otto --tasks");

    let text = String::from_utf8_lossy(&output.stdout);
    for task in ["boom", "quiet-boom", "noisy", "fanout"] {
        assert!(text.contains(task), "--tasks must list {task}: {text:?}");
    }
}
