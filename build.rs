use std::process::Command;
use std::fs;
use regex::Regex;

fn main() {
    // Ensure build.rs reruns on git changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/");
    // Get git describe output
    let git_describe = Command::new("git")
        .args(["describe", "--tags", "--long", "--always"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // Use regex to strip -0-g<sha> if present
    let version = Regex::new(r"^(.*)-0-g[0-9a-f]+$")
        .ok()
        .and_then(|re| re.captures(&git_describe))
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .unwrap_or_else(|| git_describe.clone());

    println!("cargo:rustc-env=GIT_DESCRIBE={}", version);

    // Read Cargo.toml version
    let cargo_toml = fs::read_to_string("Cargo.toml").unwrap_or_default();
    let cargo_version = cargo_toml
        .lines()
        .find(|l| l.trim_start().starts_with("version "))
        .and_then(|l| l.split('=').nth(1))
        .map(|v| v.trim().trim_matches('"'))
        .unwrap_or("");

    // Get latest tag (if any)
    let latest_tag = Command::new("git")
        .args(["describe", "--tags", "--abbrev=0"]) // only tag name
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Remove leading 'v' from tag for comparison
    let tag_version = latest_tag.strip_prefix('v').unwrap_or(&latest_tag);

    if !cargo_version.is_empty() && !tag_version.is_empty() && cargo_version != tag_version {
        println!("cargo:warning=Version mismatch: Cargo.toml version is {}, latest tag is {}", cargo_version, latest_tag);
    }
}
