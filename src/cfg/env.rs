use eyre::{Result, eyre};
use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::process::Command;

/// Evaluate environment variables with shell command substitution and variable resolution
pub fn evaluate_envs(
    envs: &HashMap<String, String>,
    working_dir: Option<&std::path::Path>,
) -> Result<HashMap<String, String>> {
    let mut evaluated = HashMap::new();
    let mut pending: Vec<String> = envs.keys().cloned().collect();
    let mut iterations = 0;
    const MAX_ITERATIONS: usize = 100; // Prevent infinite loops

    // Get current environment for variable resolution
    // Start with system environment, but input envs will override
    let mut current_env: HashMap<String, String> = env::vars().collect();

    // Remove any keys from current_env that are defined in input envs
    // This ensures input envs take precedence and prevents outer environment pollution
    for key in envs.keys() {
        current_env.remove(key);
    }

    while !pending.is_empty() && iterations < MAX_ITERATIONS {
        iterations += 1;
        let mut made_progress = false;
        let mut still_pending = Vec::new();

        for var_name in pending {
            let raw_value = envs.get(&var_name).unwrap();

            match evaluate_single_env_value(raw_value, &current_env, working_dir) {
                Ok(resolved_value) => {
                    evaluated.insert(var_name.clone(), resolved_value.clone());
                    current_env.insert(var_name, resolved_value);
                    made_progress = true;
                }
                Err(_) => {
                    // Might depend on other variables not yet resolved
                    still_pending.push(var_name);
                }
            }
        }

        if !made_progress && !still_pending.is_empty() {
            // Try to evaluate remaining variables with partial resolution
            for var_name in &still_pending {
                let raw_value = envs.get(var_name).unwrap();
                match evaluate_single_env_value(raw_value, &current_env, working_dir) {
                    Ok(resolved_value) => {
                        evaluated.insert(var_name.clone(), resolved_value.clone());
                        current_env.insert(var_name.clone(), resolved_value);
                    }
                    Err(e) => {
                        return Err(eyre!("Failed to resolve environment variable '{}': {}", var_name, e));
                    }
                }
            }
            break;
        }

        pending = still_pending;
    }

    if iterations >= MAX_ITERATIONS {
        return Err(eyre!(
            "Maximum iterations reached while resolving environment variables - possible circular dependency"
        ));
    }

    Ok(evaluated)
}

/// Evaluate a single environment variable value with shell command substitution and variable resolution
fn evaluate_single_env_value(
    value: &str,
    env_context: &HashMap<String, String>,
    working_dir: Option<&std::path::Path>,
) -> Result<String> {
    let mut result = value.to_string();

    // Step 1: Resolve shell command substitution $(...)
    // Pass env_context to prevent parent environment pollution
    result = resolve_shell_commands_with_env(&result, working_dir, env_context)?;

    result = resolve_env_variables(&result, env_context)?;

    Ok(result)
}

/// Resolve shell command substitution patterns with explicit environment
fn resolve_shell_commands_with_env(
    input: &str,
    working_dir: Option<&std::path::Path>,
    env_context: &HashMap<String, String>,
) -> Result<String> {
    let re = Regex::new(r"\$\(([^)]+)\)").unwrap();
    let mut result = input.to_string();

    for captures in re.captures_iter(input) {
        let full_match = &captures[0];
        let command_str = &captures[1];

        // Execute the shell command with controlled environment
        let output = execute_shell_command_with_env(command_str, working_dir, env_context)?;
        result = result.replace(full_match, &output);
    }

    Ok(result)
}

fn execute_shell_command_with_env(
    command_str: &str,
    working_dir: Option<&std::path::Path>,
    env_overrides: &HashMap<String, String>,
) -> Result<String> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command_str);

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    // ALWAYS use controlled environment to prevent parent process pollution
    // This is critical for preventing Otto's environment from leaking into subprocesses
    cmd.env_clear();

    let essential_vars = ["PATH", "HOME", "USER", "SHELL", "TERM", "LANG", "LC_ALL"];
    for var in &essential_vars {
        if let Ok(value) = env::var(var) {
            cmd.env(var, value);
        }
    }

    // Add the explicit environment context (variables we're building up during evaluation)
    cmd.envs(env_overrides);

    let output = cmd
        .output()
        .map_err(|e| eyre!("Failed to execute command '{}': {}", command_str, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(eyre!(
            "Command '{}' failed with exit code {}: {}",
            command_str,
            output.status.code().unwrap_or(-1),
            stderr
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().to_string())
}

/// Resolve environment variable references: ${VAR} and $VAR
fn resolve_env_variables(input: &str, env_context: &HashMap<String, String>) -> Result<String> {
    let mut result = input.to_string();

    let re_braced = Regex::new(r"\$\{([^}]+)\}").unwrap();
    for captures in re_braced.captures_iter(input) {
        let full_match = &captures[0];
        let var_name = &captures[1];

        // ONLY use env_context - never fall back to system environment
        // This ensures proper test isolation and prevents environment pollution
        let var_value = env_context
            .get(var_name)
            .ok_or_else(|| eyre!("Environment variable '{}' not found", var_name))?;

        result = result.replace(full_match, var_value);
    }

    // Handle $VAR pattern (less specific, handle after braced)
    let re_simple = Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    for captures in re_simple.captures_iter(&result.clone()) {
        let full_match = &captures[0];
        let var_name = &captures[1];

        // Skip if this is part of a ${...} pattern we already handled
        if input.contains(&format!("${{{var_name}}}")) {
            continue;
        }

        // ONLY use env_context - never fall back to system environment
        // This ensures proper test isolation and prevents environment pollution
        let var_value = env_context
            .get(var_name)
            .ok_or_else(|| eyre!("Environment variable '{}' not found", var_name))?;

        result = result.replace(full_match, var_value);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_resolve_env_variables() {
        let mut env_context = HashMap::new();
        env_context.insert("USER".to_string(), "testuser".to_string());
        env_context.insert("VERSION".to_string(), "1.0.0".to_string());

        let result = resolve_env_variables("Hello ${USER}, version is $VERSION", &env_context).unwrap();
        assert_eq!(result, "Hello testuser, version is 1.0.0");
    }

    #[test]
    fn test_resolve_shell_commands() {
        let result = resolve_shell_commands_with_env("Today is $(date +%Y-%m-%d)", None, &HashMap::new()).unwrap();
        assert!(result.starts_with("Today is 20")); // Should be a date like "Today is 2024-01-15"
    }

    #[test]
    fn test_evaluate_envs_simple() {
        let mut envs = HashMap::new();
        envs.insert("GREETING".to_string(), "Hello ${TEST_USER}".to_string());

        unsafe {
            env::set_var("TEST_USER", "testuser");
        }

        let result = evaluate_envs(&envs, None).unwrap();
        assert_eq!(result.get("GREETING").unwrap(), "Hello testuser");

        // Clean up our test variable
        unsafe {
            env::remove_var("TEST_USER");
        }
    }

    #[test]
    fn test_evaluate_envs_with_shell_command() {
        let mut envs = HashMap::new();
        envs.insert("ECHO_TEST".to_string(), "$(echo hello world)".to_string());

        let result = evaluate_envs(&envs, None).unwrap();
        assert_eq!(result.get("ECHO_TEST").unwrap(), "hello world");
    }

    #[test]
    fn test_evaluate_envs_dependency_chain() {
        let mut envs = HashMap::new();
        envs.insert("BASE".to_string(), "myapp".to_string());
        envs.insert("VERSION".to_string(), "$(echo 1.0.0)".to_string());
        envs.insert("FULL_NAME".to_string(), "${BASE}-${VERSION}".to_string());

        let result = evaluate_envs(&envs, None).unwrap();
        assert_eq!(result.get("BASE").unwrap(), "myapp");
        assert_eq!(result.get("VERSION").unwrap(), "1.0.0");
        assert_eq!(result.get("FULL_NAME").unwrap(), "myapp-1.0.0");
    }
}
