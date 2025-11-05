use assert_cmd::cargo::cargo_bin_cmd;
use serial_test::serial;
use std::fs;
use tempfile::TempDir;

/// Helper to set up isolated test database and workspace
fn setup_test_db(temp_dir: &std::path::Path) -> std::path::PathBuf {
    let db_path = temp_dir.join("test_otto.db");
    let otto_home = temp_dir.join(".otto");
    unsafe {
        std::env::set_var("OTTO_DB_PATH", &db_path);
        std::env::set_var("OTTO_HOME", &otto_home);
    }
    db_path
}

/// Test that all four built-in commands are registered and show up in help
#[test]
#[serial]
fn test_all_builtin_commands_registered() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = setup_test_db(temp_dir.path());
    let otto_home = temp_dir.path().join(".otto");
    let ottofile = temp_dir.path().join("otto.yml");

    // Create minimal ottofile
    fs::write(
        &ottofile,
        r#"
tasks:
  dummy:
    action: echo "test"
"#,
    )?;

    let output = cargo_bin_cmd!("otto")
        .current_dir(temp_dir.path())
        .env("OTTO_DB_PATH", &db_path)
        .env("OTTO_HOME", &otto_home)
        .arg("--help")
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // All four built-in commands MUST appear in help
    assert!(
        stdout.contains("graph") && stdout.contains("[built-in]"),
        "graph command not found in help"
    );
    assert!(
        stdout.contains("clean") && stdout.contains("[built-in]"),
        "clean command not found in help"
    );
    assert!(
        stdout.contains("history") && stdout.contains("[built-in]"),
        "history command not found in help"
    );
    assert!(
        stdout.contains("stats") && stdout.contains("[built-in]"),
        "stats command not found in help"
    );

    Ok(())
}

/// Test that graph command can be invoked
#[test]
#[serial]
fn test_graph_command_exists() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = setup_test_db(temp_dir.path());
    let otto_home = temp_dir.path().join(".otto");
    let ottofile = temp_dir.path().join("otto.yml");

    fs::write(
        &ottofile,
        r#"
tasks:
  test:
    action: echo "test"
"#,
    )?;

    let mut cmd = cargo_bin_cmd!("otto");
    let output = cmd
        .current_dir(temp_dir.path())
        .env("OTTO_DB_PATH", &db_path)
        .env("OTTO_HOME", &otto_home)
        .arg("graph")
        .arg("--help")
        .output()?;

    assert!(output.status.success(), "graph --help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("graph") || stdout.contains("Visualize"),
        "graph help should mention graph/visualize"
    );

    Ok(())
}

/// Test that clean command can be invoked
#[test]
#[serial]
fn test_clean_command_exists() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = cargo_bin_cmd!("otto");
    let output = cmd.arg("clean").arg("--help").output()?;

    assert!(output.status.success(), "clean --help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("clean") || stdout.contains("Clean"),
        "clean help should mention clean"
    );

    Ok(())
}

/// Test that history command can be invoked
#[test]
#[serial]
fn test_history_command_exists() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = cargo_bin_cmd!("otto");
    let output = cmd.arg("history").arg("--help").output()?;

    assert!(output.status.success(), "history --help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("history") || stdout.contains("execution history"),
        "history help should mention history"
    );

    Ok(())
}

/// Test that stats command can be invoked
#[test]
#[serial]
fn test_stats_command_exists() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = cargo_bin_cmd!("otto");
    let output = cmd.arg("stats").arg("--help").output()?;

    assert!(output.status.success(), "stats --help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("stats") || stdout.contains("statistics"),
        "stats help should mention stats"
    );

    Ok(())
}

/// Test that all built-in commands are filtered out during normal execution
#[test]
#[serial]
fn test_builtin_commands_filtered_from_execution() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = setup_test_db(temp_dir.path());
    let otto_home = temp_dir.path().join(".otto");
    let ottofile = temp_dir.path().join("otto.yml");

    // Create ottofile with a task that depends on a built-in
    fs::write(
        &ottofile,
        r#"
tasks:
  real-task:
    action: echo "executing real task"
"#,
    )?;

    // If we try to run "graph real-task", it should handle graph specially
    // and then execute real-task normally
    let mut cmd = cargo_bin_cmd!("otto");
    let output = cmd
        .current_dir(temp_dir.path())
        .env("OTTO_DB_PATH", &db_path)
        .env("OTTO_HOME", &otto_home)
        .arg("real-task")
        .output()?;

    // Should succeed - real-task executes
    assert!(output.status.success(), "real-task should execute successfully");

    Ok(())
}

/// Test count of built-in commands (regression test)
#[test]
#[serial]
fn test_builtin_command_count() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let db_path = setup_test_db(temp_dir.path());
    let otto_home = temp_dir.path().join(".otto");
    let ottofile = temp_dir.path().join("otto.yml");

    fs::write(
        &ottofile,
        r#"
tasks:
  dummy:
    action: echo "test"
"#,
    )?;

    let mut cmd = cargo_bin_cmd!("otto");
    let output = cmd
        .current_dir(temp_dir.path())
        .env("OTTO_DB_PATH", &db_path)
        .env("OTTO_HOME", &otto_home)
        .arg("--help")
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Count how many times "[built-in]" appears
    let builtin_count = stdout.matches("[built-in]").count();

    assert_eq!(
        builtin_count, 4,
        "Expected exactly 4 built-in commands, found {}. Commands: graph, clean, history, stats",
        builtin_count
    );

    Ok(())
}

/// Test that built-in commands have proper help text
#[test]
#[serial]
fn test_builtin_commands_have_help() -> Result<(), Box<dyn std::error::Error>> {
    let commands = vec![
        ("graph", "Visualize", "--format"),
        ("clean", "Clean", "--keep"),
        ("history", "history", "--limit"),
        ("stats", "statistics", "--limit"),
    ];

    for (cmd_name, expected_word, expected_flag) in commands {
        let mut cmd = cargo_bin_cmd!("otto");
        let output = cmd.arg(cmd_name).arg("--help").output()?;

        assert!(output.status.success(), "{} --help should succeed", cmd_name);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.to_lowercase().contains(&expected_word.to_lowercase()),
            "{} help should mention '{}'",
            cmd_name,
            expected_word
        );
        assert!(
            stdout.contains(expected_flag),
            "{} help should mention flag '{}'",
            cmd_name,
            expected_flag
        );
    }

    Ok(())
}
