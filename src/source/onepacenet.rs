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

use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::sync::LazyLock;
use tokio::sync::OnceCell;
use tracing::debug;

use super::DataSource;
use super::cache::CachedHttp;
use crate::matcher::normalize_arc;
use crate::model::{Episode, ImageKind, Season, Series};

const WATCH_URL: &str = "https://onepace.net/watch";

pub struct OnepaceNet {
    http: Arc<CachedHttp>,
    timeline: OnceCell<Arc<Timeline>>,
}

#[derive(Debug, Clone, Deserialize)]
struct Segment {
    #[allow(dead_code)]
    slug: String,
    title: String,
    description: String,
    special: bool,
    #[allow(dead_code)]
    chapters: Option<String>,
    #[allow(dead_code)]
    episodes: Option<String>,
}

/// Position-indexed view of the segment list: the Nth non-special segment
/// is season N. Specials are not exposed (they don't map cleanly to a
/// Jellyfin season number, and the user libraries we care about don't
/// rely on them).
#[derive(Debug)]
struct Timeline {
    seasons_by_number: Vec<(u32, Segment)>,
    by_normalized_title: std::collections::HashMap<String, u32>,
}

impl OnepaceNet {
    pub fn new(http: Arc<CachedHttp>) -> Self {
        Self {
            http,
            timeline: OnceCell::new(),
        }
    }

    async fn ensure_timeline(&self) -> Result<Arc<Timeline>> {
        self.timeline
            .get_or_try_init(|| async {
                let body = fetch_rsc(&self.http).await?;
                let segments = extract_segments(&body)?;
                Ok(Arc::new(build_timeline(segments)))
            })
            .await
            .cloned()
    }
}

async fn fetch_rsc(http: &CachedHttp) -> Result<String> {
    http.get_string_with_header(WATCH_URL, "RSC", "1")
        .await
        .context("fetching onepace.net /watch RSC payload")
}

static RSC_LINE_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9a-f]+:").unwrap());

fn extract_segments(rsc: &str) -> Result<Vec<Segment>> {
    let line = rsc
        .lines()
        .find(|l| l.contains("\"timeline\":") && l.contains("\"segments\":"))
        .ok_or_else(|| anyhow!("no timeline.segments line in RSC payload"))?;

    let payload = RSC_LINE_PREFIX_RE
        .find(line)
        .map(|m| &line[m.end()..])
        .ok_or_else(|| anyhow!("RSC line missing hex-id prefix"))?;

    let val: Value =
        serde_json::from_str(payload).context("parsing RSC JSON payload")?;
    let segments_val = find_segments(&val)
        .ok_or_else(|| anyhow!("could not locate segments[] in RSC JSON"))?;
    let segments: Vec<Segment> =
        serde_json::from_value(segments_val.clone()).context("deserializing segments")?;
    if segments.is_empty() {
        bail!("onepace.net returned empty segment list");
    }
    Ok(segments)
}

fn find_segments(v: &Value) -> Option<&Value> {
    match v {
        Value::Object(map) => {
            if let Some(s) = map.get("segments") {
                return Some(s);
            }
            for child in map.values() {
                if let Some(s) = find_segments(child) {
                    return Some(s);
                }
            }
            None
        }
        Value::Array(arr) => arr.iter().find_map(find_segments),
        _ => None,
    }
}

fn build_timeline(segments: Vec<Segment>) -> Timeline {
    let mut number = 0u32;
    let mut by_num = Vec::new();
    let mut by_title = std::collections::HashMap::new();
    for seg in segments {
        if seg.special {
            continue;
        }
        number += 1;
        by_title.insert(normalize_arc(&seg.title), number);
        by_num.push((number, seg));
    }
    Timeline {
        seasons_by_number: by_num,
        by_normalized_title: by_title,
    }
}

#[async_trait]
impl DataSource for OnepaceNet {
    fn name(&self) -> &'static str {
        "onepace.net"
    }

    async fn series(&self) -> Result<Option<Series>> {
        // onepace.net has no series-level entry; the composite falls
        // through to a source that does (SpykerNZ).
        Ok(None)
    }

    async fn season(&self, number: u32) -> Result<Option<Season>> {
        let timeline = self.ensure_timeline().await?;
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

    async fn episode(
        &self,
        _arc_normalized: &str,
        _episode_number: u32,
    ) -> Result<Option<Episode>> {
        // onepace.net treats each arc as the watchable unit; no per-episode
        // titles or plots are exposed.
        Ok(None)
    }

    async fn image(&self, kind: ImageKind) -> Result<Option<Vec<u8>>> {
        // Site provides backdrops but no clean per-season "poster" image.
        // Leave images to other sources for now.
        debug!(?kind, "onepace.net does not expose poster images");
        Ok(None)
    }
}

/// Look up the season number for a normalized arc title coming from a user
/// filename. Public for the composite to use when it wants OnepaceNet's
/// numbering directly. (Currently unused outside this module but kept for
/// the upcoming arc-name-driven episode lookups.)
#[allow(dead_code)]
pub fn season_for_arc(_arc_normalized: &str) -> Option<u32> {
    // Stub for future direct lookups; the composite uses the trait surface.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_segments_parses_real_shape() {
        // A minimal stand-in for the RSC line: <hex>:<JSON>
        let rsc = "5:[\"$\",\"$L10\",null,{\"data\":{\"timeline\":{\"segments\":[\
            {\"slug\":\"romance-dawn\",\"title\":\"Romance Dawn\",\
            \"description\":\"Luffy.\",\"special\":false,\
            \"chapters\":\"1-7\",\"episodes\":\"1-3\"},\
            {\"slug\":\"foo\",\"title\":\"Foo\",\
            \"description\":\"x.\",\"special\":true,\
            \"chapters\":\"\",\"episodes\":\"\"}\
            ]}}}]";
        let segs = extract_segments(rsc).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].slug, "romance-dawn");
        assert!(!segs[0].special);
        assert!(segs[1].special);
    }

    #[test]
    fn build_timeline_skips_specials_and_numbers_sequentially() {
        let segments = vec![
            Segment {
                slug: "a".into(),
                title: "Romance Dawn".into(),
                description: "d".into(),
                special: false,
                chapters: None,
                episodes: None,
            },
            Segment {
                slug: "s".into(),
                title: "Special".into(),
                description: "d".into(),
                special: true,
                chapters: None,
                episodes: None,
            },
            Segment {
                slug: "b".into(),
                title: "Orange Town".into(),
                description: "d".into(),
                special: false,
                chapters: None,
                episodes: None,
            },
        ];
        let tl = build_timeline(segments);
        assert_eq!(tl.seasons_by_number.len(), 2);
        assert_eq!(tl.seasons_by_number[0].0, 1);
        assert_eq!(tl.seasons_by_number[1].0, 2);
        assert_eq!(tl.by_normalized_title.get("romance dawn"), Some(&1));
        assert_eq!(tl.by_normalized_title.get("orange town"), Some(&2));
        assert_eq!(tl.by_normalized_title.get("special"), None);
    }
}
