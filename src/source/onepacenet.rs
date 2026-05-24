//! Adapter for onepace.net (the official site).
//!
//! The legacy `/api/graphql` endpoint was retired in early 2026. The current
//! site is a Next.js App Router app; the canonical arc list ships inline in
//! the RSC payload for `/watch`. We fetch that payload with the `RSC: 1`
//! header (smaller than the HTML), locate the line containing the
//! `timeline.segments` JSON, strip the `<hex>:` RSC prefix, and parse.
//!
//! Coverage: this source provides season-level metadata only (title, plot,
//! chapter range). It does NOT have per-episode titles/plots or poster
//! images — those still come from SpykerNZ via the composite source.

use std::cell::OnceCell;
use std::rc::Rc;
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow, bail};
use regex_lite::Regex;
use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

use super::cache::CachedHttp;
use super::{DataSource, ImageKind};
use crate::dl::{Release, parse_magnet};
use crate::matcher::ParsedFile;
use crate::model::{Episode, Season, Series};

const WATCH_URL: &str = "https://onepace.net/watch";
const RELEASES_URL: &str = "https://onepace.net/releases";

pub struct OnepaceNet {
    http: Rc<CachedHttp>,
    timeline: OnceCell<Rc<Timeline>>,
}

#[derive(Debug, Clone, Deserialize)]
struct Segment {
    title: String,
    description: String,
    special: bool,
}

/// Position-indexed view of the segment list: the Nth non-special segment
/// is season N. Specials are not exposed (they don't map cleanly to a
/// Jellyfin season number, and the user libraries we care about don't
/// rely on them).
#[derive(Debug)]
struct Timeline {
    seasons_by_number: Vec<(u32, Segment)>,
}

impl OnepaceNet {
    pub fn new(http: Rc<CachedHttp>) -> Self {
        Self {
            http,
            timeline: OnceCell::new(),
        }
    }

    fn ensure_timeline(&self) -> Result<Rc<Timeline>> {
        if let Some(tl) = self.timeline.get() {
            return Ok(Rc::clone(tl));
        }
        let body = fetch_rsc(&self.http)?;
        let segments = extract_segments(&body)?;
        let tl = Rc::new(build_timeline(segments));
        let _ = self.timeline.set(Rc::clone(&tl));
        Ok(Rc::clone(self.timeline.get().expect("just set")))
    }

    /// Fetch the `/releases` RSC payload and pull out every release entry.
    /// Each release becomes a [`Release`] with the raw magnet URI plus
    /// the parsed filename from the magnet's `dn=` parameter. Entries
    /// whose filename doesn't match the One Pace naming scheme are
    /// included with `parsed = None`; the caller decides whether to skip.
    pub fn fetch_releases(&self) -> Result<Vec<Release>> {
        let body = self
            .http
            .get_string_with_header(RELEASES_URL, "RSC", "1")
            .context("fetching onepace.net /releases RSC payload")?;
        Ok(extract_releases(&body))
    }
}

fn fetch_rsc(http: &CachedHttp) -> Result<String> {
    http.get_string_with_header(WATCH_URL, "RSC", "1")
        .context("fetching onepace.net /watch RSC payload")
}

static RSC_LINE_PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]+:").unwrap());

fn extract_segments(rsc: &str) -> Result<Vec<Segment>> {
    let line = rsc
        .lines()
        .find(|l| l.contains("\"timeline\":") && l.contains("\"segments\":"))
        .ok_or_else(|| anyhow!("no timeline.segments line in RSC payload"))?;

    let payload = RSC_LINE_PREFIX_RE
        .find(line)
        .map(|m| &line[m.end()..])
        .ok_or_else(|| anyhow!("RSC line missing hex-id prefix"))?;

    let val: Value = serde_json::from_str(payload).context("parsing RSC JSON payload")?;
    let segments_val =
        find_segments(&val).ok_or_else(|| anyhow!("could not locate segments[] in RSC JSON"))?;
    let segments: Vec<Segment> =
        serde_json::from_value(segments_val.clone()).context("deserializing segments")?;
    if segments.is_empty() {
        bail!("onepace.net returned empty segment list");
    }
    Ok(segments)
}

/// Locate the `segments` array anywhere in the parsed RSC payload.
///
/// Iterative DFS rather than recursive — RSC payloads are attacker-adjacent
/// (we trust onepace.net but a hostile network would have to MitM TLS to
/// alter them) and a deeply nested object should not be able to blow the
/// process stack regardless.
fn find_segments(root: &Value) -> Option<&Value> {
    let mut stack: Vec<&Value> = vec![root];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(map) => {
                if let Some(s) = map.get("segments") {
                    return Some(s);
                }
                stack.extend(map.values());
            }
            Value::Array(arr) => stack.extend(arr),
            _ => {}
        }
    }
    None
}

// ---------- releases ----------

/// Scan the RSC payload for every `T<hex>,magnet:?...` text blob and
/// turn each one into a [`Release`]. The releases chunk is one giant RSC
/// "text" chunk made of many length-prefixed blobs concatenated; some
/// blobs are magnet URIs, some are pixeldrain URLs, some are unrelated
/// strings. We only keep magnets and don't worry about which JSX node
/// each one was attached to — the filename inside `dn=` is enough.
fn extract_releases(rsc: &str) -> Vec<Release> {
    let mut releases = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cap in MAGNET_IN_RSC_RE.find_iter(rsc) {
        let uri = cap.as_str();
        // Strip any trailing artifact from the regex match (we anchor up
        // to the next `T<hex>,` chunk boundary or end of line; trim
        // anything past the last valid tracker).
        let uri = uri.trim_end_matches(|c: char| c.is_ascii_digit() || c == ':');
        let Some(parsed_magnet) = parse_magnet(uri) else {
            continue;
        };
        if !seen.insert(parsed_magnet.btih.clone()) {
            continue; // same torrent listed twice
        }
        let filename = parsed_magnet.display_name.clone().unwrap_or_default();
        let parsed = ParsedFile::from_filename(&filename);
        releases.push(Release {
            magnet: uri.to_string(),
            filename,
            parsed,
        });
    }
    releases
}

/// Matches `magnet:?...` URIs greedily up to (but not including) the next
/// chunk boundary marker `<digits>:T<hex>,` that the RSC stream uses
/// to glue together text blobs. Trailing junk gets trimmed in
/// [`extract_releases`].
static MAGNET_IN_RSC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"magnet:\?[^\s\n]+?(?:announce|$)").unwrap());

fn build_timeline(segments: Vec<Segment>) -> Timeline {
    let mut number = 0u32;
    let mut by_num = Vec::new();
    for seg in segments {
        if seg.special {
            continue;
        }
        number += 1;
        by_num.push((number, seg));
    }
    Timeline {
        seasons_by_number: by_num,
    }
}

impl DataSource for OnepaceNet {
    fn name(&self) -> &'static str {
        "onepace.net"
    }

    fn series(&self) -> Result<Option<Series>> {
        // onepace.net has no series-level entry; the composite falls
        // through to a source that does (SpykerNZ).
        Ok(None)
    }

    fn season(&self, number: u32) -> Result<Option<Season>> {
        let timeline = self.ensure_timeline()?;
        let Some((_, seg)) = timeline
            .seasons_by_number
            .iter()
            .find(|(n, _)| *n == number)
        else {
            return Ok(None);
        };
        Ok(Some(Season {
            number,
            title: format!("{}. {}", number, seg.title),
            plot: Some(seg.description.clone()),
        }))
    }

    fn episode(&self, _arc_normalized: &str, _episode_number: u32) -> Result<Option<Episode>> {
        // onepace.net treats each arc as the watchable unit; no per-episode
        // titles or plots are exposed.
        Ok(None)
    }

    fn image(&self, kind: ImageKind) -> Result<Option<Vec<u8>>> {
        // Site provides backdrops but no clean per-season "poster" image.
        // Leave images to other sources for now.
        debug!(?kind, "onepace.net does not expose poster images");
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_segments_parses_real_shape() {
        // A minimal stand-in for the RSC line: <hex>:<JSON>
        let rsc = "5:[\"$\",\"$L10\",null,{\"data\":{\"timeline\":{\"segments\":[\
            {\"title\":\"Romance Dawn\",\"description\":\"Luffy.\",\"special\":false},\
            {\"title\":\"Foo\",\"description\":\"x.\",\"special\":true}\
            ]}}}]";
        let segs = extract_segments(rsc).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].title, "Romance Dawn");
        assert!(!segs[0].special);
        assert!(segs[1].special);
    }

    fn seg(title: &str, special: bool) -> Segment {
        Segment {
            title: title.into(),
            description: "d".into(),
            special,
        }
    }

    #[test]
    fn build_timeline_skips_specials_and_numbers_sequentially() {
        let tl = build_timeline(vec![
            seg("Romance Dawn", false),
            seg("Special", true),
            seg("Orange Town", false),
        ]);
        assert_eq!(tl.seasons_by_number.len(), 2);
        assert_eq!(tl.seasons_by_number[0].0, 1);
        assert_eq!(tl.seasons_by_number[0].1.title, "Romance Dawn");
        assert_eq!(tl.seasons_by_number[1].0, 2);
        assert_eq!(tl.seasons_by_number[1].1.title, "Orange Town");
    }
}
