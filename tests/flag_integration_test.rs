use std::fs;
use tempfile::TempDir;
use otto::cli::parser::Parser;

#[test]
fn test_boolean_flags_integration() {
    let temp_dir = TempDir::new().unwrap();
    let otto_file = temp_dir.path().join("otto.yml");

    let config = r#"
otto:
  api: 1
  tasks: [build]

tasks:
  build:
    params:
      -v|--verbose:
        default: false
        help: Enable verbose output
      -f|--force:
        default: false
        help: Force rebuild
      --dry-run:
        default: false
        help: Show what would be done
    action: |
      #!/bin/bash
      echo "Verbose: ${verbose}"
      echo "Force: ${force}"
      echo "Dry run: ${dry_run}"
    "#;

    fs::write(&otto_file, config).unwrap();

    // Test with flags present
    let args = vec![
        "otto".to_string(),
        "-o".to_string(),
        otto_file.to_string_lossy().to_string(),
        "build".to_string(),
        "--verbose".to_string(),
        "--force".to_string(),
        "--dry-run".to_string(),
    ];

    let mut parser = Parser::new(args).unwrap();
    let (tasks, _, _) = parser.parse().unwrap();

    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.name, "build");
    assert_eq!(task.envs.get("verbose").unwrap(), "true");
    assert_eq!(task.envs.get("force").unwrap(), "true");
    assert_eq!(task.envs.get("dry_run").unwrap(), "true");
}

#[test]
fn test_boolean_flags_absent_integration() {
    let temp_dir = TempDir::new().unwrap();
    let otto_file = temp_dir.path().join("otto.yml");

    let config = r#"
otto:
  api: 1
  tasks: [build]

tasks:
  build:
    params:
      -v|--verbose:
        default: false
        help: Enable verbose output
      -f|--force:
        default: false
        help: Force rebuild
    action: |
      #!/bin/bash
      echo "Verbose: ${verbose}"
      echo "Force: ${force}"
    "#;

    fs::write(&otto_file, config).unwrap();

    // Test with no flags (should use defaults)
    let args = vec![
        "otto".to_string(),
        "-o".to_string(),
        otto_file.to_string_lossy().to_string(),
        "build".to_string(),
    ];

    let mut parser = Parser::new(args).unwrap();
    let (tasks, _, _) = parser.parse().unwrap();

    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.name, "build");
    assert_eq!(task.envs.get("verbose").unwrap(), "false");
    assert_eq!(task.envs.get("force").unwrap(), "false");
}

#[test]
fn test_argument_flags_integration() {
    let temp_dir = TempDir::new().unwrap();
    let otto_file = temp_dir.path().join("otto.yml");

    let config = r#"
otto:
  api: 1
  tasks: [deploy]

tasks:
  deploy:
    params:
      -e|--env:
        default: development
        choices: [development, staging, production]
        help: Target environment
      --timeout:
        default: 30
        help: Timeout in seconds
      -p|--port:
        default: 8080
        help: Port number
    action: |
      #!/bin/bash
      echo "Environment: ${env}"
      echo "Timeout: ${timeout}"
      echo "Port: ${port}"
    "#;

    fs::write(&otto_file, config).unwrap();

    // Test with explicit values
    let args = vec![
        "otto".to_string(),
        "-o".to_string(),
        otto_file.to_string_lossy().to_string(),
        "deploy".to_string(),
        "--env".to_string(),
        "production".to_string(),
        "--timeout".to_string(),
        "60".to_string(),
        "-p".to_string(),
        "3000".to_string(),
    ];

    let mut parser = Parser::new(args).unwrap();
    let (tasks, _, _) = parser.parse().unwrap();

    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.name, "deploy");
    assert_eq!(task.envs.get("env").unwrap(), "production");
    assert_eq!(task.envs.get("timeout").unwrap(), "60");
    assert_eq!(task.envs.get("port").unwrap(), "3000");
}

#[test]
fn test_argument_flags_with_defaults_integration() {
    let temp_dir = TempDir::new().unwrap();
    let otto_file = temp_dir.path().join("otto.yml");

    let config = r#"
otto:
  api: 1
  tasks: [serve]

tasks:
  serve:
    params:
      -p|--port:
        default: 3000
        help: Port number
      --host:
        default: localhost
        help: Host address
      -w|--workers:
        default: 4
        help: Number of workers
    action: |
      #!/bin/bash
      echo "Port: ${port}"
      echo "Host: ${host}"
      echo "Workers: ${workers}"
    "#;

    fs::write(&otto_file, config).unwrap();

    // Test with some explicit values and some defaults
    let args = vec![
        "otto".to_string(),
        "-o".to_string(),
        otto_file.to_string_lossy().to_string(),
        "serve".to_string(),
        "--port".to_string(),
        "8080".to_string(),
        // host and workers should use defaults
    ];

    let mut parser = Parser::new(args).unwrap();
    let (tasks, _, _) = parser.parse().unwrap();

    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.name, "serve");
    assert_eq!(task.envs.get("port").unwrap(), "8080");
    assert_eq!(task.envs.get("host").unwrap(), "localhost");
    assert_eq!(task.envs.get("workers").unwrap(), "4");
}

#[test]
fn test_mixed_flags_integration() {
    let temp_dir = TempDir::new().unwrap();
    let otto_file = temp_dir.path().join("otto.yml");

    let config = r#"
otto:
  api: 1
  tasks: [test]

tasks:
  test:
    params:
      # Boolean flags
      -v|--verbose:
        default: false
        help: Enable verbose output
      --coverage:
        default: false
        help: Generate coverage report
      --watch:
        default: false
        help: Watch for file changes

      # Argument flags
      -p|--pattern:
        default: "**/*.test.js"
        help: Test file pattern
      --reporter:
        choices: [spec, json, junit, tap]
        default: spec
        help: Test reporter format
      --timeout:
        default: 5000
        help: Test timeout in milliseconds
    action: |
      #!/bin/bash
      echo "Verbose: ${verbose}"
      echo "Coverage: ${coverage}"
      echo "Watch: ${watch}"
      echo "Pattern: ${pattern}"
      echo "Reporter: ${reporter}"
      echo "Timeout: ${timeout}"
    "#;

    fs::write(&otto_file, config).unwrap();

    let args = vec![
        "otto".to_string(),
        "-o".to_string(),
        otto_file.to_string_lossy().to_string(),
        "test".to_string(),
        "--verbose".to_string(),
        "--coverage".to_string(),
        "--pattern".to_string(),
        "src/**/*.test.js".to_string(),
        "--reporter".to_string(),
        "json".to_string(),
        "--timeout".to_string(),
        "10000".to_string(),
        // watch should default to false
    ];

    let mut parser = Parser::new(args).unwrap();
    let (tasks, _, _) = parser.parse().unwrap();

    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.name, "test");

    // Boolean flags
    assert_eq!(task.envs.get("verbose").unwrap(), "true");
    assert_eq!(task.envs.get("coverage").unwrap(), "true");
    assert_eq!(task.envs.get("watch").unwrap(), "false");

    // Argument flags
    assert_eq!(task.envs.get("pattern").unwrap(), "src/**/*.test.js");
    assert_eq!(task.envs.get("reporter").unwrap(), "json");
    assert_eq!(task.envs.get("timeout").unwrap(), "10000");
}

#[test]
fn test_short_flag_combinations() {
    let temp_dir = TempDir::new().unwrap();
    let otto_file = temp_dir.path().join("otto.yml");

    let config = r#"
otto:
  api: 1
  tasks: [build]

tasks:
  build:
    params:
      -v|--verbose:
        default: false
        help: Enable verbose output
      -f|--force:
        default: false
        help: Force rebuild
      -q|--quiet:
        default: false
        help: Quiet output
      -e|--env:
        default: development
        choices: [development, staging, production]
        help: Target environment
    action: |
      #!/bin/bash
      echo "Building..."
    "#;

    fs::write(&otto_file, config).unwrap();

    let args = vec![
        "otto".to_string(),
        "-o".to_string(),
        otto_file.to_string_lossy().to_string(),
        "build".to_string(),
        "-v".to_string(),
        "-f".to_string(),
        "-e".to_string(),
        "production".to_string(),
    ];

    let mut parser = Parser::new(args).unwrap();
    let (tasks, _, _) = parser.parse().unwrap();

    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.name, "build");
    assert_eq!(task.envs.get("verbose").unwrap(), "true");
    assert_eq!(task.envs.get("force").unwrap(), "true");
    assert_eq!(task.envs.get("quiet").unwrap(), "false");
    assert_eq!(task.envs.get("env").unwrap(), "production");
}