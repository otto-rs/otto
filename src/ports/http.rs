//! HTTP abstraction for dependency injection.
//!
//! This module provides traits for HTTP operations, enabling testing
//! with mock implementations instead of making real network requests.

use async_trait::async_trait;
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Release information from GitHub API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub name: String,
    pub published_at: String,
    pub assets: Vec<AssetInfo>,
}

/// Asset information from a release
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInfo {
    pub name: String,
    pub browser_download_url: String,
}

/// Trait for fetching release information from a remote source.
///
/// This abstraction allows testing upgrade logic without making real HTTP requests.
#[async_trait]
pub trait ReleaseFetcher: Send + Sync {
    /// Fetch all available releases
    async fn fetch_releases(&self, repo: &str, github_token: Option<&str>) -> Result<Vec<ReleaseInfo>>;

    /// Download an asset to a temporary file, returning the path
    async fn download_asset(&self, url: &str) -> Result<PathBuf>;
}

/// Real implementation that makes HTTP requests via reqwest
pub struct HttpReleaseFetcher;

impl HttpReleaseFetcher {
    pub fn new() -> Self {
        HttpReleaseFetcher
    }
}

impl Default for HttpReleaseFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ReleaseFetcher for HttpReleaseFetcher {
    async fn fetch_releases(&self, repo: &str, github_token: Option<&str>) -> Result<Vec<ReleaseInfo>> {
        use eyre::{Context, eyre};
        use reqwest::Client;

        let client = Client::new();
        let url = format!("https://api.github.com/repos/{}/releases", repo);

        let mut request = client.get(&url).header("User-Agent", "otto-upgrade");

        if let Some(token) = github_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request.send().await.context("Failed to fetch releases from GitHub")?;

        if !response.status().is_success() {
            return Err(eyre!(
                "GitHub API returned error: {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }

        let releases: Vec<ReleaseInfo> = response.json().await.context("Failed to parse GitHub releases")?;

        Ok(releases)
    }

    async fn download_asset(&self, url: &str) -> Result<PathBuf> {
        use eyre::Context;
        use futures_util::StreamExt;
        use reqwest::Client;
        use std::fs::File;
        use std::io::Write;

        let client = Client::new();
        let response = client.get(url).send().await?;

        let temp_dir = tempfile::tempdir()?;
        let file_path = temp_dir.keep().join("otto.tar.gz");
        let mut file = File::create(&file_path).context("Failed to create temp file")?;

        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk)?;
        }

        Ok(file_path)
    }
}

/// Mock implementation for testing
pub struct MockReleaseFetcher {
    releases: Arc<Mutex<Vec<ReleaseInfo>>>,
    download_content: Arc<Mutex<Option<Vec<u8>>>>,
    download_path: Arc<Mutex<Option<PathBuf>>>,
}

impl MockReleaseFetcher {
    pub fn new() -> Self {
        MockReleaseFetcher {
            releases: Arc::new(Mutex::new(Vec::new())),
            download_content: Arc::new(Mutex::new(None)),
            download_path: Arc::new(Mutex::new(None)),
        }
    }

    /// Set the releases that will be returned by fetch_releases
    pub fn with_releases(self, releases: Vec<ReleaseInfo>) -> Self {
        *self.releases.lock().unwrap() = releases;
        self
    }

    /// Set content to write when download_asset is called
    pub fn with_download_content(self, content: Vec<u8>) -> Self {
        *self.download_content.lock().unwrap() = Some(content);
        self
    }

    /// Set a specific path to return from download_asset (for testing)
    pub fn with_download_path(self, path: PathBuf) -> Self {
        *self.download_path.lock().unwrap() = Some(path);
        self
    }

    /// Add a release to the mock
    pub fn add_release(&self, release: ReleaseInfo) {
        self.releases.lock().unwrap().push(release);
    }

    /// Get the releases currently configured
    pub fn get_releases(&self) -> Vec<ReleaseInfo> {
        self.releases.lock().unwrap().clone()
    }
}

impl Default for MockReleaseFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ReleaseFetcher for MockReleaseFetcher {
    async fn fetch_releases(&self, _repo: &str, _github_token: Option<&str>) -> Result<Vec<ReleaseInfo>> {
        Ok(self.releases.lock().unwrap().clone())
    }

    async fn download_asset(&self, _url: &str) -> Result<PathBuf> {
        // If a specific path is set, return it
        if let Some(path) = self.download_path.lock().unwrap().clone() {
            return Ok(path);
        }

        // Otherwise create a temp file with the configured content
        let temp_dir = tempfile::tempdir()?;
        let file_path = temp_dir.keep().join("otto.tar.gz");

        if let Some(content) = self.download_content.lock().unwrap().as_ref() {
            std::fs::write(&file_path, content)?;
        } else {
            // Create empty file
            std::fs::write(&file_path, b"")?;
        }

        Ok(file_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_release(version: &str) -> ReleaseInfo {
        ReleaseInfo {
            tag_name: format!("v{}", version),
            name: format!("Otto v{}", version),
            published_at: "2024-01-15T12:00:00Z".to_string(),
            assets: vec![
                AssetInfo {
                    name: format!("otto-v{}-linux.tar.gz", version),
                    browser_download_url: format!("https://example.com/otto-v{}-linux.tar.gz", version),
                },
                AssetInfo {
                    name: format!("otto-v{}-macos-arm64.tar.gz", version),
                    browser_download_url: format!("https://example.com/otto-v{}-macos-arm64.tar.gz", version),
                },
            ],
        }
    }

    #[test]
    fn test_mock_fetcher_new() {
        let fetcher = MockReleaseFetcher::new();
        assert!(fetcher.get_releases().is_empty());
    }

    #[test]
    fn test_mock_fetcher_with_releases() {
        let releases = vec![sample_release("0.5.6"), sample_release("0.5.5")];
        let fetcher = MockReleaseFetcher::new().with_releases(releases.clone());

        assert_eq!(fetcher.get_releases().len(), 2);
        assert_eq!(fetcher.get_releases()[0].tag_name, "v0.5.6");
    }

    #[test]
    fn test_mock_fetcher_add_release() {
        let fetcher = MockReleaseFetcher::new();
        fetcher.add_release(sample_release("0.5.6"));
        fetcher.add_release(sample_release("0.5.5"));

        assert_eq!(fetcher.get_releases().len(), 2);
    }

    #[tokio::test]
    async fn test_mock_fetch_releases() {
        let releases = vec![sample_release("0.5.6"), sample_release("0.5.5")];
        let fetcher = MockReleaseFetcher::new().with_releases(releases);

        let result = fetcher.fetch_releases("scottidler/otto", None).await.unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].tag_name, "v0.5.6");
        assert_eq!(result[0].assets.len(), 2);
    }

    #[tokio::test]
    async fn test_mock_download_empty() {
        let fetcher = MockReleaseFetcher::new();

        let path = fetcher.download_asset("https://example.com/file.tar.gz").await.unwrap();

        assert!(path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"");
    }

    #[tokio::test]
    async fn test_mock_download_with_content() {
        let content = b"fake tarball content".to_vec();
        let fetcher = MockReleaseFetcher::new().with_download_content(content.clone());

        let path = fetcher.download_asset("https://example.com/file.tar.gz").await.unwrap();

        assert!(path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), content);
    }

    #[tokio::test]
    async fn test_mock_download_with_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let expected_path = temp_dir.path().join("test.tar.gz");
        std::fs::write(&expected_path, b"test content").unwrap();

        let fetcher = MockReleaseFetcher::new().with_download_path(expected_path.clone());

        let path = fetcher.download_asset("https://example.com/file.tar.gz").await.unwrap();

        assert_eq!(path, expected_path);
    }

    #[test]
    fn test_release_info_serialization() {
        let release = sample_release("1.0.0");
        let json = serde_json::to_string(&release).unwrap();
        let parsed: ReleaseInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tag_name, "v1.0.0");
        assert_eq!(parsed.assets.len(), 2);
    }

    #[test]
    fn test_http_fetcher_default() {
        let _fetcher = HttpReleaseFetcher;
        // Just verify it compiles and doesn't panic
    }

    #[test]
    fn test_mock_fetcher_default() {
        let fetcher = MockReleaseFetcher::default();
        assert!(fetcher.get_releases().is_empty());
    }
}
