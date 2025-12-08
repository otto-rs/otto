/// Integration tests for example otto.yml files
/// These tests ensure examples stay working as the codebase evolves
use assert_cmd::Command;

/// Helper to get the otto binary path
#[allow(deprecated)]
fn otto_cmd() -> Command {
    Command::cargo_bin("otto").expect("Failed to find otto binary")
}

/// Helper to run an example and verify it succeeds
fn run_example(example_dir: &str, task: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = otto_cmd();
    cmd.current_dir(format!("examples/{}", example_dir));
    cmd.arg(task);
    
    let output = cmd.output()?;
    
    if !output.status.success() {
        eprintln!("=== STDOUT ===");
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("=== STDERR ===");
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        return Err(format!("Example {} failed with exit code: {:?}", example_dir, output.status.code()).into());
    }
    
    Ok(())
}

/// Helper to just validate an example parses correctly
fn validate_example_parses(example_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = otto_cmd();
    cmd.current_dir(format!("examples/{}", example_dir));
    cmd.arg("--help");
    
    let output = cmd.output()?;
    
    if !output.status.success() {
        eprintln!("=== STDERR ===");
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        return Err(format!("Example {} failed to parse", example_dir).into());
    }
    
    // Should show task list in help
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Commands:") || stdout.contains("Usage:"), 
            "Help output should show commands for {}", example_dir);
    
    Ok(())
}

// ============================================================================
// Simple Examples - Just verify they parse
// ============================================================================

#[test]
fn test_ex1_parses() {
    validate_example_parses("ex1").expect("ex1 should parse");
}

#[test]
fn test_ex2_parses() {
    validate_example_parses("ex2").expect("ex2 should parse");
}

#[test]
fn test_ex3_parses() {
    validate_example_parses("ex3").expect("ex3 should parse");
}

#[test]
fn test_ex4_parses() {
    validate_example_parses("ex4").expect("ex4 should parse");
}

#[test]
fn test_ex5_parses() {
    validate_example_parses("ex5").expect("ex5 should parse");
}

#[test]
fn test_ex6_parses() {
    validate_example_parses("ex6").expect("ex6 should parse");
}

#[test]
fn test_ex7_parses() {
    validate_example_parses("ex7").expect("ex7 should parse");
}

#[test]
fn test_ex8_parses() {
    validate_example_parses("ex8").expect("ex8 should parse");
}

#[test]
fn test_ex9_parses() {
    validate_example_parses("ex9").expect("ex9 should parse");
}

#[test]
fn test_ex10_parses() {
    validate_example_parses("ex10").expect("ex10 should parse");
}

#[test]
fn test_ex12_parses() {
    validate_example_parses("ex12").expect("ex12 should parse");
}

#[test]
fn test_ex13_parses() {
    validate_example_parses("ex13").expect("ex13 should parse");
}

// ============================================================================
// Examples That Should Execute Successfully
// ============================================================================

#[test]
fn test_ex1_punch_executes() {
    run_example("ex1", "punch").expect("ex1 punch should execute successfully");
}

#[test]
fn test_ex11_data_passing() {
    // This tests the data passing mechanism (bash only)
    run_example("ex11", "consume").expect("ex11 consume should execute successfully");
}

#[test]
fn test_ex14_data_passing_validation() {
    // This tests data passing with bash and python
    run_example("ex14", "report").expect("ex14 report should execute successfully");
}

// ============================================================================
// Real-world Examples - Verify they parse
// ============================================================================

#[test]
fn test_auth_svc_parses() {
    validate_example_parses("auth-svc").expect("auth-svc should parse");
}

#[test]
fn test_devs_parses() {
    validate_example_parses("devs").expect("devs should parse");
}

#[test]
fn test_pre_commit_hooks_parses() {
    validate_example_parses("pre-commit-hooks").expect("pre-commit-hooks should parse");
}

#[test]
fn test_media_planning_service_parses() {
    validate_example_parses("media-planning-service").expect("media-planning-service should parse");
}

// ============================================================================
// Special Examples - Just verify they parse (can't run without interaction)
// ============================================================================

#[test]
fn test_interactive_demo_parses() {
    validate_example_parses("interactive-demo").expect("interactive-demo should parse");
}

#[test]
fn test_tui_demo_parses() {
    validate_example_parses("tui-demo").expect("tui-demo should parse");
}

// ============================================================================
// Negative Test - Verify we catch broken examples
// ============================================================================

#[test]
#[should_panic(expected = "should parse")]
fn test_validates_broken_examples() {
    // This would fail if we had a broken example (good!)
    // Create a temporary broken example
    std::fs::create_dir_all("examples/_test_broken").ok();
    std::fs::write("examples/_test_broken/otto.yml", "invalid: yaml: content: [[[").ok();
    
    validate_example_parses("_test_broken").expect("should parse");
    
    // Cleanup
    std::fs::remove_dir_all("examples/_test_broken").ok();
}

