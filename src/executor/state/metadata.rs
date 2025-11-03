use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Metadata about a run, stored in run.yaml and the database
/// This struct is shared between the file-based system and SQLite
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RunMetadata {
    /// Path to the ottofile used for this run
    pub ottofile: Option<PathBuf>,

    /// Project hash (e.g., "6b20a2e4" from otto-6b20a2e4/)
    #[serde(default)]
    pub hash: String,

    /// Unix timestamp when run started (also used as directory name)
    #[serde(default)]
    pub timestamp: u64,

    /// Current working directory when run was executed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,

    /// Username who executed the run
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    /// Hostname where run was executed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,

    /// Command-line arguments (serialized as JSON string in DB)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
}

impl RunMetadata {
    /// Create minimal metadata (for backward compatibility with existing run.yaml files)
    pub fn minimal(ottofile: Option<PathBuf>, hash: String, timestamp: u64) -> Self {
        Self {
            ottofile,
            hash,
            timestamp,
            cwd: None,
            user: None,
            hostname: None,
            args: None,
        }
    }

    pub fn full(
        ottofile: Option<PathBuf>,
        hash: String,
        timestamp: u64,
        cwd: Option<PathBuf>,
        user: Option<String>,
        hostname: Option<String>,
        args: Option<Vec<String>>,
    ) -> Self {
        Self {
            ottofile,
            hash,
            timestamp,
            cwd,
            user,
            hostname,
            args,
        }
    }

    /// Get current system metadata (user, hostname)
    pub fn current_system_info() -> (Option<String>, Option<String>) {
        let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).ok();

        let hostname = hostname::get().ok().and_then(|h| h.into_string().ok());

        (user, hostname)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minimal_metadata() {
        let meta = RunMetadata::minimal(Some(PathBuf::from("/test/otto.yml")), "abc123".to_string(), 1234567890);

        assert_eq!(meta.ottofile, Some(PathBuf::from("/test/otto.yml")));
        assert_eq!(meta.hash, "abc123");
        assert_eq!(meta.timestamp, 1234567890);
        assert_eq!(meta.cwd, None);
        assert_eq!(meta.user, None);
        assert_eq!(meta.hostname, None);
        assert_eq!(meta.args, None);
    }

    #[test]
    fn test_full_metadata() {
        let meta = RunMetadata::full(
            Some(PathBuf::from("/test/otto.yml")),
            "abc123".to_string(),
            1234567890,
            Some(PathBuf::from("/home/user/project")),
            Some("testuser".to_string()),
            Some("testhost".to_string()),
            Some(vec!["build".to_string(), "test".to_string()]),
        );

        assert_eq!(meta.ottofile, Some(PathBuf::from("/test/otto.yml")));
        assert_eq!(meta.hash, "abc123");
        assert_eq!(meta.timestamp, 1234567890);
        assert_eq!(meta.cwd, Some(PathBuf::from("/home/user/project")));
        assert_eq!(meta.user, Some("testuser".to_string()));
        assert_eq!(meta.hostname, Some("testhost".to_string()));
        assert_eq!(meta.args, Some(vec!["build".to_string(), "test".to_string()]));
    }

    #[test]
    fn test_serde_roundtrip() {
        let meta = RunMetadata::full(
            Some(PathBuf::from("/test/otto.yml")),
            "abc123".to_string(),
            1234567890,
            Some(PathBuf::from("/home/user/project")),
            Some("testuser".to_string()),
            Some("testhost".to_string()),
            Some(vec!["build".to_string()]),
        );

        let yaml = serde_yaml::to_string(&meta).unwrap();
        let parsed: RunMetadata = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(meta, parsed);
    }

    #[test]
    fn test_backward_compatible_minimal_yaml() {
        // Test that we can parse old run.yaml files that only have minimal fields
        let yaml = r#"
ottofile: /test/otto.yml
hash: abc123
timestamp: 1234567890
"#;

        let parsed: RunMetadata = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.ottofile, Some(PathBuf::from("/test/otto.yml")));
        assert_eq!(parsed.hash, "abc123");
        assert_eq!(parsed.timestamp, 1234567890);
        assert_eq!(parsed.cwd, None);
    }

    #[test]
    fn test_current_system_info() {
        let (user, hostname) = RunMetadata::current_system_info();

        // At least one should be available on most systems
        assert!(user.is_some() || hostname.is_some());
    }
}
