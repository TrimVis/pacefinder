//! On-disk HTTP cache keyed by URL hash.
//!
//! Simple TTL cache: every GET writes the response body to
//! `~/.cache/pacefinder/http/<sha256(url)>.bin`. Subsequent reads return
//! the file if it's younger than `ttl`. Bypass the cache with
//! [`CachedHttp::refresh`].

use anyhow::{Context, Result};
use directories::ProjectDirs;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tracing::{debug, trace};
use ureq::Agent;

pub struct CachedHttp {
    agent: Agent,
    cache_dir: PathBuf,
    ttl: Duration,
    refresh: bool,
}

/// On-disk location of the HTTP cache, independent of whether a
/// [`CachedHttp`] has ever been constructed. Used by the `cache` subcommand.
pub fn cache_dir() -> Result<PathBuf> {
    let dirs =
        ProjectDirs::from("net", "PaceFinder", "pacefinder").context("resolving project dirs")?;
    Ok(dirs.cache_dir().join("http"))
}

/// Delete every cached body. The directory itself is left in place.
pub fn clear() -> Result<()> {
    let dir = cache_dir()?;
    if !dir.exists() {
        return Ok(());
    }
    let mut removed = 0usize;
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    tracing::info!(dir = %dir.display(), removed, "cleared cache");
    Ok(())
}

impl CachedHttp {
    pub fn new(ttl: Duration) -> Result<Self> {
        let dirs = ProjectDirs::from("net", "PaceFinder", "pacefinder")
            .context("resolving project dirs")?;
        let cache_dir = dirs.cache_dir().join("http");
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("creating cache dir {}", cache_dir.display()))?;
        let agent: Agent = Agent::config_builder()
            .user_agent(concat!("pacefinder/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Ok(Self {
            agent,
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

    pub fn get_string_with_header(&self, url: &str, name: &str, value: &str) -> Result<String> {
        let bytes = self.get_bytes_with(url, &[(name, value)])?;
        String::from_utf8(bytes).context("response body not utf-8")
    }

    pub fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        self.get_bytes_with(url, &[])
    }

    fn get_bytes_with(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        let path = self.path_for_keyed(url, headers);

        if !self.refresh && self.is_fresh(&path) {
            trace!(%url, "cache hit");
            return fs::read(&path).with_context(|| format!("reading cache {}", path.display()));
        }

        debug!(%url, "fetching");
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let mut resp = req.call().with_context(|| format!("GET {url}"))?;
        let bytes = resp
            .body_mut()
            .read_to_vec()
            .with_context(|| format!("body for {url}"))?;
        fs::write(&path, &bytes).with_context(|| format!("writing cache {}", path.display()))?;
        Ok(bytes)
    }

    pub fn get_string(&self, url: &str) -> Result<String> {
        let bytes = self.get_bytes(url)?;
        String::from_utf8(bytes).context("response body not utf-8")
    }

    fn is_fresh(&self, path: &Path) -> bool {
        let Ok(meta) = fs::metadata(path) else {
            return false;
        };
        let Ok(modified) = meta.modified() else {
            return false;
        };
        // Clock skew (NTP correction, container time jumps) can leave
        // mtimes in the future; treat those as age 0 rather than stale.
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or(Duration::ZERO);
        age < self.ttl
    }

    fn path_for_keyed(&self, url: &str, headers: &[(&str, &str)]) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        for (k, v) in headers {
            hasher.update(b"\0");
            hasher.update(k.as_bytes());
            hasher.update(b":");
            hasher.update(v.as_bytes());
        }
        let digest = hex::encode(hasher.finalize());
        self.cache_dir.join(format!("{digest}.bin"))
    }
}
