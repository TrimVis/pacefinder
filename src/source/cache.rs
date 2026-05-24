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

    #[cfg(test)]
    fn for_test(cache_dir: PathBuf, ttl: Duration) -> Self {
        Self {
            agent: Agent::config_builder().build().into(),
            cache_dir,
            ttl,
            refresh: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        File::create(&p).unwrap();
        p
    }

    #[test]
    fn is_fresh_true_for_recent_file() {
        let dir = tempdir().unwrap();
        let p = touch(dir.path(), "a.bin");
        let http = CachedHttp::for_test(dir.path().to_path_buf(), Duration::from_secs(300));
        assert!(http.is_fresh(&p));
    }

    #[test]
    fn is_fresh_false_for_stale_file() {
        let dir = tempdir().unwrap();
        let p = touch(dir.path(), "a.bin");
        File::open(&p)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        let http = CachedHttp::for_test(dir.path().to_path_buf(), Duration::from_secs(60));
        assert!(!http.is_fresh(&p));
    }

    #[test]
    fn is_fresh_true_for_future_mtime() {
        // Bug-fix regression: clock skew (NTP, container time jump) used
        // to flip the file to stale and force a re-download.
        let dir = tempdir().unwrap();
        let p = touch(dir.path(), "a.bin");
        File::open(&p)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(60))
            .unwrap();
        let http = CachedHttp::for_test(dir.path().to_path_buf(), Duration::from_secs(300));
        assert!(http.is_fresh(&p), "future mtime should count as fresh");
    }

    #[test]
    fn is_fresh_false_for_missing_file() {
        let dir = tempdir().unwrap();
        let http = CachedHttp::for_test(dir.path().to_path_buf(), Duration::from_secs(60));
        assert!(!http.is_fresh(&dir.path().join("not-here.bin")));
    }

    #[test]
    fn path_for_keyed_distinguishes_url_header_and_value() {
        let dir = tempdir().unwrap();
        let http = CachedHttp::for_test(dir.path().to_path_buf(), Duration::from_secs(60));
        let plain_a = http.path_for_keyed("https://a", &[]);
        let plain_b = http.path_for_keyed("https://b", &[]);
        assert_ne!(plain_a, plain_b, "different URLs → different paths");

        let with_hdr = http.path_for_keyed("https://a", &[("RSC", "1")]);
        assert_ne!(plain_a, with_hdr, "added header → different path");

        let with_other_value = http.path_for_keyed("https://a", &[("RSC", "2")]);
        assert_ne!(with_hdr, with_other_value, "header value matters");

        let same_again = http.path_for_keyed("https://a", &[("RSC", "1")]);
        assert_eq!(with_hdr, same_again, "same key → same path");
    }
}
