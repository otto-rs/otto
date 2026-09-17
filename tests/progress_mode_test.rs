//! Phase 3 of `docs/design/2026-09-16-live-progress-renderer.md`: mode
//! plumbing and config migration. No renderer exists yet - these tests cover
//! what IS observable from outside the binary at this phase: `--progress` /
//! `OTTO_PROGRESS` / `otto.progress` accept `auto`/`never` and reject
//! anything else cleanly, `OTTO_PROGRESS` is stripped from a nested otto's
//! environment, and `progress-interval` still loads and now warns.

mod common;

use common::{OTTO_BIN, isolate};
use std::fs;
use std::process::Stdio;
use tempfile::TempDir;

const OTTOFILE: &str = r#"
tasks:
  quiet:
    bash: echo START
"#;

fn project() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("otto.yml"), OTTOFILE).expect("write ottofile");
    let home = dir.path().join("otto-home");
    fs::create_dir_all(&home).expect("create otto home");
    (dir, home)
}

fn run(args: &[&str], extra_env: &[(&str, &str)]) -> (bool, i32, String) {
    let (dir, home) = project();
    let mut cmd = std::process::Command::new(OTTO_BIN);
    cmd.args(args);
    isolate(&mut cmd, &home);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let output = cmd
        .current_dir(dir.path())
        .env_remove("OTTOFILE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run otto");

    let mut merged = String::from_utf8_lossy(&output.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), output.status.code().unwrap_or(-1), merged)
}

#[test]
fn progress_auto_is_accepted() {
    let (ok, _code, out) = run(&["--progress", "auto", "quiet"], &[]);
    assert!(ok, "auto must be accepted:\n{out}");
    assert!(out.contains("START"), "the task must still run:\n{out}");
}

#[test]
fn progress_never_is_accepted() {
    let (ok, _code, out) = run(&["--progress", "never", "quiet"], &[]);
    assert!(ok, "never must be accepted:\n{out}");
    assert!(out.contains("START"), "the task must still run:\n{out}");
}

/// `always` is cut (Phase 0, `docs/design/2026-09-16-live-progress-renderer.md`
/// API Design). A user who read cargo's `term.progress.when` docs may still
/// type it, and it must fail with a clean, named error - otto's normal exit
/// code 1 for a rejected `ParseOutcome` (see `main.rs`), not a panic (101)
/// and not a silent fallback to `auto` (which would exit 0 and run the task).
#[test]
fn progress_always_is_rejected_on_the_cli_not_silently_accepted() {
    let (ok, code, out) = run(&["--progress", "always", "quiet"], &[]);
    assert!(!ok, "always must be rejected, not silently run:\n{out}");
    assert_eq!(code, 1, "a validation error, not a panic (101):\n{out}");
    assert!(out.contains("always"), "error should name the rejected value:\n{out}");
    assert!(
        out.contains("auto") && out.contains("never"),
        "error should name the valid values:\n{out}"
    );
    assert!(!out.contains("START"), "the task must not have run:\n{out}");
}

/// Same rejection, through `OTTO_PROGRESS` instead of the flag. clap applies
/// the same `PossibleValuesParser` to a value sourced from the env.
#[test]
fn progress_always_is_rejected_through_the_env_var_too() {
    let (ok, code, out) = run(&["quiet"], &[("OTTO_PROGRESS", "always")]);
    assert!(!ok, "OTTO_PROGRESS=always must be rejected:\n{out}");
    assert_eq!(code, 1, "a validation error, not a panic (101):\n{out}");
    assert!(out.contains("always"), "error should name the rejected value:\n{out}");
}

/// Success criterion: an ottofile carrying `progress-interval` loads
/// successfully and warns.
#[test]
fn an_ottofile_carrying_progress_interval_loads_and_warns() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("otto.yml"),
        "otto:\n  progress-interval: 5\ntasks:\n  quiet:\n    bash: echo START\n",
    )
    .expect("write ottofile");
    let home = dir.path().join("otto-home");
    fs::create_dir_all(&home).expect("create otto home");

    let mut cmd = std::process::Command::new(OTTO_BIN);
    cmd.arg("quiet");
    isolate(&mut cmd, &home);
    let output = cmd
        .current_dir(dir.path())
        .env_remove("OTTOFILE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run otto");

    assert!(output.status.success(), "the ottofile must still load and run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("START"), "the task must still run:\n{stdout}");
    assert!(
        stderr.contains("deprecated"),
        "otto.progress-interval must warn on stderr:\n{stderr}"
    );
}

/// Success criterion: a nested otto sees no `OTTO_PROGRESS` in its
/// environment. The outer process is launched with `OTTO_PROGRESS=never`;
/// the outer task's body invokes otto again (against the same ottofile, via
/// cwd search) on an `inner` task that echoes its own `OTTO_PROGRESS`. If the
/// strip at `scheduler/task_execution.rs` regressed, the nested run would
/// read back `never` instead of finding the variable absent.
#[test]
fn a_nested_otto_sees_no_otto_progress_in_its_environment() {
    let dir = TempDir::new().expect("tempdir");
    let ottofile = "tasks:\n  inner:\n    bash: |\n      echo \"INNER_SAW=${OTTO_PROGRESS:-ABSENT}\"\n  outer:\n    bash: |\n      \"$OTTO_BIN_UNDER_TEST\" inner\n";
    fs::write(dir.path().join("otto.yml"), ottofile).expect("write ottofile");
    let home = dir.path().join("otto-home");
    fs::create_dir_all(&home).expect("create otto home");

    let mut cmd = std::process::Command::new(OTTO_BIN);
    cmd.arg("outer");
    isolate(&mut cmd, &home);
    let output = cmd
        .current_dir(dir.path())
        .env_remove("OTTOFILE")
        .env("OTTO_PROGRESS", "never")
        .env("OTTO_BIN_UNDER_TEST", OTTO_BIN)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run otto");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the nested run must succeed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("INNER_SAW=ABSENT"),
        "the nested otto's own child task must not inherit the parent's OTTO_PROGRESS:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
