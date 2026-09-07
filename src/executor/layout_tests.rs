#![cfg(test)]

use super::*;
use serial_test::serial;
use std::path::PathBuf;

#[test]
fn test_project_dir_name_is_name_then_hash() {
    assert_eq!(project_dir_name("otto", "6b20a2e4"), "otto-6b20a2e4");
}

#[test]
fn test_run_dir_joins_home_project_timestamp() {
    let dir = run_dir(Path::new("/home/u/.otto"), "otto", "6b20a2e4", 1700000000);
    assert_eq!(dir, PathBuf::from("/home/u/.otto/otto-6b20a2e4/1700000000"));
}

#[test]
fn test_parse_round_trips_what_the_workspace_builds() {
    let name = project_dir_name("my-project", "0123abcd");
    assert_eq!(parse_project_dir_name(&name), Some(("my-project", "0123abcd")));
}

#[test]
fn test_parse_rejects_non_project_entries() {
    // The things that actually sit next to run roots in the otto home.
    assert_eq!(parse_project_dir_name("otto.db"), None);
    assert_eq!(parse_project_dir_name(".cache"), None);
    assert_eq!(parse_project_dir_name(".last_prune"), None);
    // A hash of the wrong length or the wrong alphabet is not a run root.
    assert_eq!(parse_project_dir_name("proj-abc123"), None);
    assert_eq!(parse_project_dir_name("proj-ABCDEF12"), None);
    assert_eq!(parse_project_dir_name("proj-6b20a2eg"), None);
    assert_eq!(parse_project_dir_name("-6b20a2e4"), None);
}

#[test]
#[serial]
fn test_resolve_otto_home_prefers_otto_home() {
    // SAFETY: serialized against every other env-mutating test in the crate.
    unsafe {
        std::env::set_var("OTTO_HOME", "/tmp/otto-home-probe");
    }
    let home = resolve_otto_home().unwrap();
    unsafe {
        std::env::remove_var("OTTO_HOME");
    }
    assert_eq!(home, PathBuf::from("/tmp/otto-home-probe"));
}

#[test]
#[serial]
fn test_resolve_otto_home_falls_back_to_dot_otto() {
    // SAFETY: serialized against every other env-mutating test in the crate.
    unsafe {
        std::env::remove_var("OTTO_HOME");
    }
    let home = resolve_otto_home().unwrap();
    let expected = PathBuf::from(std::env::var("HOME").unwrap()).join(".otto");
    assert_eq!(home, expected);
}

#[test]
#[serial]
fn xdg_data_dir_honors_the_env_and_falls_back() {
    let prior = std::env::var("XDG_DATA_HOME").ok();

    let dir = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("XDG_DATA_HOME", dir.path()) };
    assert_eq!(xdg_data_dir().as_deref(), Some(dir.path()));
    assert_eq!(log_dir(), Some(dir.path().join("otto").join("logs")));

    // A relative value is not a usable data dir; fall back rather than
    // scattering logs relative to whatever the cwd happened to be.
    unsafe { std::env::set_var("XDG_DATA_HOME", "relative/path") };
    assert!(xdg_data_dir().expect("a home dir").ends_with(".local/share"));

    unsafe { std::env::remove_var("XDG_DATA_HOME") };
    assert!(xdg_data_dir().expect("a home dir").ends_with(".local/share"));

    match prior {
        Some(v) => unsafe { std::env::set_var("XDG_DATA_HOME", v) },
        None => unsafe { std::env::remove_var("XDG_DATA_HOME") },
    }
}

// expand_tilde is now re-exported from the `expand-tilde` crate, which
// carries its own equivalent bare/prefixed/other-user coverage.

#[test]
fn directory_size_sums_files_across_nested_directories() {
    let temp = tempfile::TempDir::new().unwrap();
    let nested = temp.path().join("tasks").join("lint");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(temp.path().join("run.yaml"), vec![0u8; 100]).unwrap();
    std::fs::write(nested.join("stdout.log"), vec![0u8; 900]).unwrap();

    assert_eq!(directory_size(temp.path()).unwrap(), 1000);
}

#[test]
fn directory_size_does_not_count_a_symlink_or_its_target() {
    let temp = tempfile::TempDir::new().unwrap();
    let outside = temp.path().join("outside");
    let run = temp.path().join("run");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(&run).unwrap();

    std::fs::write(outside.join("blob"), vec![0u8; 4096]).unwrap();
    std::fs::write(run.join("run.yaml"), vec![0u8; 100]).unwrap();
    std::os::unix::fs::symlink(outside.join("blob"), run.join("cache")).unwrap();

    // The 4096-byte target belongs to whoever owns `outside`, not to this run.
    assert_eq!(directory_size(&run).unwrap(), 100);
}

#[test]
fn directory_size_of_a_missing_path_is_zero() {
    let temp = tempfile::TempDir::new().unwrap();
    assert_eq!(directory_size(&temp.path().join("never-created")).unwrap(), 0);
}
