//! On-disk HTTP cache keyed by URL hash.
//!
//! Simple TTL cache: every GET writes the response body to
//! `~/.cache/pacefinder/http/<sha256(url)>.bin`. Subsequent reads return
//! the file if it's younger than `ttl`. Bypass the cache with
//! [`CachedHttp::refresh`].

use anyhow::{Context, Result};
use directories::ProjectDirs;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs;
use tracing::{debug, trace};

pub struct CachedHttp {
    client: Client,
    cache_dir: PathBuf,
    ttl: Duration,
    refresh: bool,
}

impl CachedHttp {
    pub fn new(ttl: Duration) -> Result<Self> {
        let dirs = ProjectDirs::from("net", "PaceFinder", "pacefinder")
            .context("resolving project dirs")?;
        let cache_dir = dirs.cache_dir().join("http");
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("creating cache dir {}", cache_dir.display()))?;
        let client = Client::builder()
            .user_agent(concat!("pacefinder/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("building http client")?;
        Ok(Self {
            client,
            cache_dir,
            ttl,
            refresh: false,
        })
    }

    /// Force a refresh on the next GET, bypassing any cached body.
    pub fn refresh(mut self, refresh: bool) -> Self {
        self.refresh = refresh;
        self
    }

    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let path = self.path_for(url);

        if !self.refresh && self.is_fresh(&path).await {
            trace!(%url, "cache hit");
            return fs::read(&path)
                .await
                .with_context(|| format!("reading cache {}", path.display()));
        }

        debug!(%url, "fetching");
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("status for {url}"))?;
        let bytes = resp
            .bytes()
            .await
            .with_context(|| format!("body for {url}"))?;
        fs::write(&path, &bytes)
            .await
            .with_context(|| format!("writing cache {}", path.display()))?;
        Ok(bytes.to_vec())
    }

    pub async fn get_string(&self, url: &str) -> Result<String> {
        let bytes = self.get_bytes(url).await?;
        String::from_utf8(bytes).context("response body not utf-8")
    }

    async fn is_fresh(&self, path: &PathBuf) -> bool {
        let Ok(meta) = fs::metadata(path).await else {
            return false;
        };
        let Ok(modified) = meta.modified() else {
            return false;
        };
        modified.elapsed().map(|e| e < self.ttl).unwrap_or(false)
    }

    fn path_for(&self, url: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        let digest = hex::encode(hasher.finalize());
        self.cache_dir.join(format!("{digest}.bin"))
    }
}
