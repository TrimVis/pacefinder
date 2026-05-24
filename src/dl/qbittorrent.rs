//! qBittorrent Web API client — minimal slice for the `download` subcommand.

use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;
use ureq::Agent;

pub struct QbtClient {
    agent: Agent,
    base: String,
    sid: String,
}

impl QbtClient {
    /// `base` is the Web UI URL with no path (e.g. `http://localhost:8080`).
    pub fn login(base: &str, user: &str, pass: &str) -> Result<Self> {
        let base = base.trim_end_matches('/').to_string();
        let agent: Agent = Agent::config_builder()
            .user_agent(concat!("pacefinder/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();

        let url = format!("{base}/api/v2/auth/login");
        // qBittorrent expects form-encoded credentials and a Referer that
        // matches the base URL (its CSRF guard).
        let body = format!("username={}&password={}", urlencode(user), urlencode(pass));
        let mut resp = agent
            .post(&url)
            .header("Referer", &base)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(body.as_bytes())
            .with_context(|| format!("POST {url}"))?;

        let sid = extract_sid_cookie(&resp).ok_or_else(|| {
            anyhow!("qbittorrent login: no SID cookie in response (bad credentials?)")
        })?;
        // Some qBittorrent builds set SID even on lockout — confirm the
        // body. Successful auth is the literal "Ok.".
        let body = resp
            .body_mut()
            .read_to_string()
            .context("reading login response body")?;
        if body.trim().eq_ignore_ascii_case("Fails.") {
            bail!("qbittorrent login: server returned 'Fails.' (wrong credentials or lockout)");
        }
        Ok(Self { agent, base, sid })
    }

    pub fn list_torrent_names(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/v2/torrents/info", self.base);
        let mut resp = self
            .agent
            .get(&url)
            .header("Cookie", format!("SID={}", self.sid))
            .call()
            .with_context(|| format!("GET {url}"))?;
        let body = resp
            .body_mut()
            .read_to_string()
            .context("reading torrents/info body")?;
        let arr: Vec<serde_json::Value> =
            serde_json::from_str(&body).context("parsing torrents/info JSON")?;
        Ok(arr
            .into_iter()
            .map(|v| {
                v.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .to_string()
            })
            .collect())
    }

    /// Used by the caller to spot mismatched mount tables between
    /// pacefinder's host and qBittorrent's container.
    pub fn default_save_path(&self) -> Result<String> {
        let url = format!("{}/api/v2/app/defaultSavePath", self.base);
        let mut resp = self
            .agent
            .get(&url)
            .header("Cookie", format!("SID={}", self.sid))
            .call()
            .with_context(|| format!("GET {url}"))?;
        let body = resp
            .body_mut()
            .read_to_string()
            .context("reading defaultSavePath body")?;
        Ok(body.trim().to_string())
    }

    pub fn add_magnet(&self, magnet: &str, save_path: &Path, category: Option<&str>) -> Result<()> {
        let url = format!("{}/api/v2/torrents/add", self.base);
        let mut body = format!(
            "urls={}&savepath={}&autoTMM=false",
            urlencode(magnet),
            urlencode(&save_path.to_string_lossy()),
        );
        if let Some(cat) = category {
            body.push_str("&category=");
            body.push_str(&urlencode(cat));
        }
        let mut resp = self
            .agent
            .post(&url)
            .header("Cookie", format!("SID={}", self.sid))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(body.as_bytes())
            .with_context(|| format!("POST {url}"))?;
        // qBittorrent returns "Ok." on success, "Fails." on error.
        let status = resp.status();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        if !status.is_success() || text.trim().eq_ignore_ascii_case("Fails.") {
            bail!("qbittorrent torrents/add failed (HTTP {status}): {text}");
        }
        Ok(())
    }
}

fn extract_sid_cookie(resp: &ureq::http::Response<ureq::Body>) -> Option<String> {
    for value in resp.headers().get_all("set-cookie") {
        let s = value.to_str().ok()?;
        for part in s.split(';') {
            if let Some(v) = part.trim().strip_prefix("SID=").filter(|v| !v.is_empty()) {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Minimal percent-encode for `application/x-www-form-urlencoded` values:
/// space → `+`, anything not in the unreserved set → `%XX`.
fn urlencode(input: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        match b {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            other => write!(out, "%{other:02X}").expect("writing to String"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_handles_typical_chars() {
        assert_eq!(urlencode("hello world"), "hello+world");
        assert_eq!(urlencode("a/b"), "a%2Fb");
        assert_eq!(urlencode("simple-thing.txt"), "simple-thing.txt");
        assert_eq!(urlencode("=&%"), "%3D%26%25");
    }

    #[test]
    fn urlencode_round_trips_through_decoder() {
        let original = "[One Pace] Romance Dawn 01 [1080p][D767799C].mkv";
        let encoded = urlencode(original);
        let decoded = super::super::urldecode_plus(&encoded);
        assert_eq!(decoded, original);
    }
}
