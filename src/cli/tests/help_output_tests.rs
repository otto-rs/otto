use std::fs;
use std::process::Command;
use std::path::PathBuf;
use tempfile::TempDir;

fn get_otto_binary_path() -> PathBuf {
    let current_dir = std::env::current_dir().unwrap();
    current_dir.join("target/debug/otto")
}

/// Test that basic help output matches exactly
#[test]
fn test_help_output_basic() {
    let temp_dir = TempDir::new().unwrap();
    let otto_path = get_otto_binary_path();
    
    let output = Command::new(&otto_path)
        .args(&["--help"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let expected = r#"A task runner

Usage: otto [OPTIONS] [COMMAND]

Options:
  -o, --ottofile <PATH>    path to the ottofile [default: ./]
  -a, --api <URL>          api url [default: 1]
  -j, --jobs <JOBS>        number of jobs to run in parallel [default: 32]
  -H, --home <PATH>        path to the Otto home directory [default: ~/.otto]
  -t, --tasks <TASKS>      comma separated list of tasks to run [default: *]
  -v, --verbosity <LEVEL>  verbosity level [default: 1]
  -V, --version            Print version

Logs are written to: ~/.local/share/otto/logs/otto.log"#;

    // Replace the dynamic log path with the expected one for comparison
    let normalized_stdout = stdout.replace(
        &format!("Logs are written to: {}", 
            dirs::data_local_dir()
                .map(|dir| dir.join("otto").join("logs").join("otto.log"))
                .and_then(|path| path.to_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "~/.local/share/otto/logs/otto.log".to_string())
        ),
        "Logs are written to: ~/.local/share/otto/logs/otto.log"
    );

    // Strip ANSI escape sequences for comparison
    let normalized_stdout = String::from_utf8_lossy(&strip_ansi_escapes::strip(&normalized_stdout))
        .to_string();

    // Check character by character for exact match
    assert_eq!(normalized_stdout.trim(), expected.trim(), 
        "Basic help output doesn't match expected format");
}

/// Test that help output with error message appears when task fails to find ottofile
#[test]
fn test_help_output_with_missing_ottofile_error() {
    let temp_dir = TempDir::new().unwrap();
    let otto_path = get_otto_binary_path();
    
    let output = Command::new(&otto_path)
        .args(&["nonexistent_task"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let expected = r#"A task runner

Usage: otto [OPTIONS] [COMMAND]

Options:
  -o, --ottofile <PATH>    path to the ottofile [default: ./]
  -a, --api <URL>          api url [default: 1]
  -j, --jobs <JOBS>        number of jobs to run in parallel [default: 32]
  -H, --home <PATH>        path to the Otto home directory [default: ~/.otto]
  -t, --tasks <TASKS>      comma separated list of tasks to run [default: *]
  -v, --verbosity <LEVEL>  verbosity level [default: 1]
  -V, --version            Print version

Logs are written to: ~/.local/share/otto/logs/otto.log

ERROR: No ottofile found in this directory or any parent directory!
Otto looks for one of the following files in the current or parent directories:

To get started, create an otto.yml file in your project root.
  - otto.yml
  - .otto.yml
  - otto.yaml
  - .otto.yaml
  - Ottofile
  - OTTOFILE"#;

    // Replace the dynamic log path with the expected one for comparison
    let normalized_stdout = stdout.replace(
        &format!("Logs are written to: {}", 
            dirs::data_local_dir()
                .map(|dir| dir.join("otto").join("logs").join("otto.log"))
                .and_then(|path| path.to_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "~/.local/share/otto/logs/otto.log".to_string())
        ),
        "Logs are written to: ~/.local/share/otto/logs/otto.log"
    );

    // Strip ANSI escape sequences for comparison
    let normalized_stdout = String::from_utf8_lossy(&strip_ansi_escapes::strip(&normalized_stdout))
        .to_string();

    // Check character by character for exact match
    assert_eq!(normalized_stdout.trim(), expected.trim(), 
        "Help output with missing ottofile error doesn't match expected format");
    
    // Should exit with code 2
    assert_eq!(output.status.code(), Some(2), "Should exit with code 2 when ottofile not found");
}

/// Test that basic help output when ottofile exists
#[test]
fn test_help_output_with_ottofile() {
    let temp_dir = TempDir::new().unwrap();
    let otto_file = temp_dir.path().join("otto.yml");
    let otto_path = get_otto_binary_path();
    
    // Create a minimal otto.yml file
    fs::write(&otto_file, r#"
otto:
  api: 1
  tasks:
    - hello
    - world
tasks:
  hello:
    help: "Say hello"
    action: |
      echo "hello"
  world:
    help: "Say world"
    action: |
      echo "world"
"#).unwrap();

    let output = Command::new(&otto_path)
        .args(&["--help"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let expected = r#"A task runner

Usage: otto [OPTIONS] [COMMAND]

Options:
  -o, --ottofile <PATH>    path to the ottofile [default: ./]
  -a, --api <URL>          api url [default: 1]
  -j, --jobs <JOBS>        number of jobs to run in parallel [default: 32]
  -H, --home <PATH>        path to the Otto home directory [default: ~/.otto]
  -t, --tasks <TASKS>      comma separated list of tasks to run [default: *]
  -v, --verbosity <LEVEL>  verbosity level [default: 1]
  -V, --version            Print version

Logs are written to: ~/.local/share/otto/logs/otto.log"#;

    // Replace the dynamic log path with the expected one for comparison
    let normalized_stdout = stdout.replace(
        &format!("Logs are written to: {}", 
            dirs::data_local_dir()
                .map(|dir| dir.join("otto").join("logs").join("otto.log"))
                .and_then(|path| path.to_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "~/.local/share/otto/logs/otto.log".to_string())
        ),
        "Logs are written to: ~/.local/share/otto/logs/otto.log"
    );

    // Strip ANSI escape sequences for comparison
    let normalized_stdout = String::from_utf8_lossy(&strip_ansi_escapes::strip(&normalized_stdout))
        .to_string();

    // Check character by character for exact match
    assert_eq!(normalized_stdout.trim(), expected.trim(), 
        "Help output with ottofile doesn't match expected format");
}

/// Test that short help flag (-h) produces same output as --help
#[test]
fn test_short_help_flag() {
    let temp_dir = TempDir::new().unwrap();
    let otto_path = get_otto_binary_path();
    
    let output_long = Command::new(&otto_path)
        .args(&["--help"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to execute command");

    let output_short = Command::new(&otto_path)
        .args(&["-h"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to execute command");

    let stdout_long = String::from_utf8(output_long.stdout).unwrap();
    let stdout_short = String::from_utf8(output_short.stdout).unwrap();

    assert_eq!(stdout_long, stdout_short, 
        "Short help flag (-h) should produce same output as --help");
}

/// Test that version flag produces correct output
#[test]
fn test_version_output() {
    let temp_dir = TempDir::new().unwrap();
    let otto_path = get_otto_binary_path();
    
    let output = Command::new(&otto_path)
        .args(&["--version"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8(output.stdout).unwrap();
    
    // Version should start with "otto " followed by version info
    assert!(stdout.starts_with("otto "), 
        "Version output should start with 'otto ', got: {}", stdout);
    
    // Should be a single line
    assert_eq!(stdout.lines().count(), 1, 
        "Version output should be a single line");
}

/// Test that short version flag (-V) produces same output as --version
#[test]
fn test_short_version_flag() {
    let temp_dir = TempDir::new().unwrap();
    let otto_path = get_otto_binary_path();
    
    let output_long = Command::new(&otto_path)
        .args(&["--version"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to execute command");

    let output_short = Command::new(&otto_path)
        .args(&["-V"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to execute command");

    let stdout_long = String::from_utf8(output_long.stdout).unwrap();
    let stdout_short = String::from_utf8(output_short.stdout).unwrap();

    assert_eq!(stdout_long, stdout_short, 
        "Short version flag (-V) should produce same output as --version");
}

/// Test that help preserves exact ANSI escape sequences for formatting
#[test]
fn test_help_ansi_formatting() {
    let temp_dir = TempDir::new().unwrap();
    let otto_path = get_otto_binary_path();
    
    let output = Command::new(&otto_path)
        .args(&["--help"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8(output.stdout).unwrap();
    
    // Check for bold formatting on "Usage:" and "Options:"
    assert!(stdout.contains("\x1b[1mUsage:\x1b[0m"), 
        "Help should contain bold formatting for 'Usage:'");
    assert!(stdout.contains("\x1b[1mOptions:\x1b[0m"), 
        "Help should contain bold formatting for 'Options:'");
    
    // Check for bold formatting on option names
    assert!(stdout.contains("\x1b[1m-o, --ottofile <PATH>\x1b[0m"), 
        "Help should contain bold formatting for option names");
}

/// Test that help error message preserves exact ANSI escape sequences for formatting
#[test]
fn test_help_error_ansi_formatting() {
    let temp_dir = TempDir::new().unwrap();
    let otto_path = get_otto_binary_path();
    
    let output = Command::new(&otto_path)
        .args(&["nonexistent_task"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8(output.stdout).unwrap();
    
    // Check for error coloring when no ottofile found
    assert!(stdout.contains("\x1b[1m\x1b[31mERROR:"), 
        "Help should contain red bold formatting for error message");
    assert!(stdout.contains("\x1b[33mOtto looks for"), 
        "Help should contain yellow formatting for informational text");
} 