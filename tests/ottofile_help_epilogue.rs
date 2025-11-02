use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_help_epilogue_when_ottofile_missing() {
    let temp = tempdir().unwrap();
    let mut cmd = cargo_bin_cmd!("otto");
    cmd.current_dir(&temp).arg("--help");
    cmd.assert()
        .failure()
        .code(2)
        .stdout(predicate::str::contains(
            "ERROR: No ottofile found in this directory or any parent directory!",
        ))
        .stdout(predicate::str::contains("Otto looks for one of the following files"))
        .stdout(predicate::str::contains("otto.yml"));
}

#[test]
fn test_help_epilogue_not_present_when_ottofile_exists() {
    let temp = tempdir().unwrap();
    let ottofile_path = temp.path().join("otto.yml");
    let mut file = fs::File::create(&ottofile_path).unwrap();
    writeln!(file, "otto:\n  api: 1\ntasks:\n  test:\n    action: echo test").unwrap();

    let mut cmd = cargo_bin_cmd!("otto");
    cmd.current_dir(&temp).arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("otto").and(predicate::str::contains("ERROR: No ottofile found").not()));
}
