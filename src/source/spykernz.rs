//! Adapter for the community-maintained SpykerNZ One Pace dataset.
//!
//! Builds an in-memory index of NFO and poster paths from one GitHub Trees
//! API call, then resolves individual files by raw URL through [`CachedHttp`].

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;
use tokio::sync::OnceCell;
use tracing::{debug, warn};

use super::DataSource;
use super::cache::CachedHttp;
use crate::matcher::normalize_arc;
use crate::model::{Episode, ImageKind, Season, Series};
use crate::nfo::kodi;

const TREE_URL: &str =
    "https://api.github.com/repos/SpykerNZ/one-pace-for-plex/git/trees/main?recursive=1";
const RAW_BASE: &str =
    "https://raw.githubusercontent.com/SpykerNZ/one-pace-for-plex/main/";

pub struct SpykerNz {
    http: Arc<CachedHttp>,
    index: OnceCell<Arc<Index>>,
    series: OnceCell<Arc<Series>>,
}

impl SpykerNz {
    pub fn new(http: Arc<CachedHttp>) -> Self {
        Self {
            http,
            index: OnceCell::new(),
            series: OnceCell::new(),
        }
    }

    async fn ensure_index(&self) -> Result<Arc<Index>> {
        self.index
            .get_or_try_init(|| async {
                let json = self.http.get_string(TREE_URL).await?;
                let tree: GitHubTree =
                    serde_json::from_str(&json).context("parsing tree response")?;
                if tree.truncated {
                    warn!("github tree response truncated — index may be incomplete");
                }
                Ok(Arc::new(build_index(&tree.tree)))
            })
            .await
            .cloned()
    }

    async fn cached_series(&self) -> Result<Arc<Series>> {
        self.series
            .get_or_try_init(|| async {
                let series = self.fetch_series().await?;
                Ok(Arc::new(series))
            })
            .await
            .cloned()
    }

    async fn fetch_series(&self) -> Result<Series> {
        let index = self.ensure_index().await?;
        let path = index
            .series_nfo
            .as_deref()
            .ok_or_else(|| anyhow!("series tvshow.nfo not in SpykerNZ index"))?;
        let xml = self.http.get_string(&raw_url(path)).await?;
        Ok(kodi::parse_tvshow(&xml)?.into())
    }

    /// Look up the season number for a normalized arc name. Tries the name
    /// as-is first, then a small set of known spelling aliases (the user
    /// community uses both "Whiskey Peak" and "Whisky Peak", for example).
    async fn season_for_arc(&self, arc_norm: &str) -> Result<Option<u32>> {
        let series = self.cached_series().await?;
        let lookup = |needle: &str| -> Option<u32> {
            series
                .named_seasons
                .iter()
                .find(|ns| normalize_arc(&strip_leading_number(&ns.name)) == needle)
                .map(|ns| ns.number)
        };
        if let Some(num) = lookup(arc_norm) {
            return Ok(Some(num));
        }
        if let Some(alias) = arc_alias(arc_norm) {
            return Ok(lookup(alias));
        }
        Ok(None)
    }
}

fn strip_leading_number(name: &str) -> &str {
    // SpykerNZ stores arc names like "1. Romance Dawn"; users see "Romance Dawn".
    name.split_once(". ").map(|(_, rest)| rest).unwrap_or(name)
}

/// Map a user-side arc name (normalized) to the SpykerNZ canonical spelling
/// when they diverge. Add entries as community-discovered.
fn arc_alias(normalized: &str) -> Option<&'static str> {
    match normalized {
        "whiskey peak" => Some("whisky peak"),
        _ => None,
    }
}

fn raw_url(path: &str) -> String {
    format!("{RAW_BASE}{}", encode_path(path))
}

fn encode_path(path: &str) -> String {
    // Raw GitHub serves filenames containing spaces, commas, apostrophes; only
    // spaces strictly need percent-encoding for URL parsing.
    path.replace(' ', "%20")
}

#[async_trait]
impl DataSource for SpykerNz {
    fn name(&self) -> &'static str {
        "SpykerNZ"
    }

    async fn series(&self) -> Result<Option<Series>> {
        let arc = self.cached_series().await?;
        Ok(Some((*arc).clone()))
    }

    async fn season(&self, number: u32) -> Result<Option<Season>> {
        let index = self.ensure_index().await?;
        let Some(season) = index.seasons.get(&number) else {
            return Ok(None);
        };
        let Some(nfo_path) = season.season_nfo.as_deref() else {
            return Ok(None);
        };
        let xml = self.http.get_string(&raw_url(nfo_path)).await?;
        Ok(Some(kodi::parse_season(&xml)?.into()))
    }

    async fn episode(
        &self,
        arc_normalized: &str,
        episode_number: u32,
    ) -> Result<Option<Episode>> {
        let Some(season_num) = self.season_for_arc(arc_normalized).await? else {
            debug!(arc = %arc_normalized, "no SpykerNZ season for arc");
            return Ok(None);
        };
        let index = self.ensure_index().await?;
        let Some(season) = index.seasons.get(&season_num) else {
            return Ok(None);
        };
        let Some(nfo_path) = season.episodes.get(&episode_number) else {
            debug!(
                season = season_num,
                episode = episode_number,
                "no SpykerNZ episode NFO at that slot"
            );
            return Ok(None);
        };
        let xml = self.http.get_string(&raw_url(nfo_path)).await?;
        Ok(Some(kodi::parse_episode(&xml)?.into()))
    }

    async fn image(&self, kind: ImageKind) -> Result<Option<Vec<u8>>> {
        let index = self.ensure_index().await?;
        let path = match kind {
            ImageKind::SeriesPoster => index.series_poster.clone(),
            ImageKind::SeasonPoster { number } => index
                .seasons
                .get(&number)
                .and_then(|s| s.season_poster.clone()),
        };
        let Some(path) = path else {
            return Ok(None);
        };
        let bytes = self.http.get_bytes(&raw_url(&path)).await?;
        Ok(Some(bytes))
    }
}

// ---------- index ----------

#[derive(Debug, Default, Clone)]
pub struct Index {
    pub series_nfo: Option<String>,
    pub series_poster: Option<String>,
    pub seasons: HashMap<u32, SeasonEntry>,
}

#[derive(Debug, Default, Clone)]
pub struct SeasonEntry {
    pub season_nfo: Option<String>,
    pub season_poster: Option<String>,
    pub episodes: HashMap<u32, String>,
}

#[derive(Debug, Deserialize)]
struct GitHubTree {
    tree: Vec<GitHubTreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubTreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
}

static SEASON_POSTER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^One Pace/season(\d+)-poster\.[a-z]+$").unwrap());
static SEASON_NFO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^One Pace/Season (\d+)/season\.nfo$").unwrap());
static EPISODE_NFO_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^One Pace/Season (\d+)/One Pace - S\d+E(\d+) - .+\.nfo$").unwrap()
});

fn build_index(entries: &[GitHubTreeEntry]) -> Index {
    let mut idx = Index::default();
    for entry in entries {
        if entry.kind != "blob" {
            continue;
        }
        let path = entry.path.as_str();

        if path == "One Pace/tvshow.nfo" {
            idx.series_nfo = Some(path.to_string());
        } else if path == "One Pace/poster.png" {
            idx.series_poster = Some(path.to_string());
        } else if let Some(caps) = SEASON_POSTER_RE.captures(path) {
            let num: u32 = caps[1].parse().unwrap_or(0);
            idx.seasons.entry(num).or_default().season_poster = Some(path.to_string());
        } else if let Some(caps) = SEASON_NFO_RE.captures(path) {
            let num: u32 = caps[1].parse().unwrap_or(0);
            idx.seasons.entry(num).or_default().season_nfo = Some(path.to_string());
        } else if let Some(caps) = EPISODE_NFO_RE.captures(path) {
            let season: u32 = caps[1].parse().unwrap_or(0);
            let episode: u32 = caps[2].parse().unwrap_or(0);
            idx.seasons
                .entry(season)
                .or_default()
                .episodes
                .insert(episode, path.to_string());
        }
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> GitHubTreeEntry {
        GitHubTreeEntry {
            path: path.to_string(),
            kind: "blob".into(),
        }
    }

    #[test]
    fn index_classifies_all_known_paths() {
        let entries = vec![
            entry("One Pace/tvshow.nfo"),
            entry("One Pace/poster.png"),
            entry("One Pace/season01-poster.png"),
            entry("One Pace/season02-poster.png"),
            entry("One Pace/Season 1/season.nfo"),
            entry("One Pace/Season 1/One Pace - S01E01 - Romance Dawn, the Dawn of an Adventure.nfo"),
            entry("One Pace/Season 1/One Pace - S01E02 - They Call Him 'Straw Hat' Luffy.nfo"),
            entry("One Pace/Season 2/season.nfo"),
            entry("README.md"),
        ];
        let idx = build_index(&entries);
        assert_eq!(idx.series_nfo.as_deref(), Some("One Pace/tvshow.nfo"));
        assert_eq!(idx.series_poster.as_deref(), Some("One Pace/poster.png"));
        assert_eq!(idx.seasons.len(), 2);

        let s1 = idx.seasons.get(&1).unwrap();
        assert!(s1.season_nfo.is_some());
        assert!(s1.season_poster.is_some());
        assert_eq!(s1.episodes.len(), 2);
        assert!(s1.episodes.contains_key(&1));
        assert!(s1.episodes.contains_key(&2));

        let s2 = idx.seasons.get(&2).unwrap();
        assert!(s2.season_nfo.is_some());
        assert_eq!(s2.season_poster.as_deref(), Some("One Pace/season02-poster.png"));
        assert!(s2.episodes.is_empty());
    }

    #[test]
    fn ignores_tree_entries() {
        let entries = vec![GitHubTreeEntry {
            path: "One Pace".into(),
            kind: "tree".into(),
        }];
        assert!(build_index(&entries).series_nfo.is_none());
    }

    #[test]
    fn encode_path_quotes_spaces_only() {
        assert_eq!(
            encode_path("One Pace/Season 1/season.nfo"),
            "One%20Pace/Season%201/season.nfo"
        );
    }

    #[test]
    fn strip_leading_number_drops_dotted_prefix() {
        assert_eq!(strip_leading_number("1. Romance Dawn"), "Romance Dawn");
        assert_eq!(strip_leading_number("plain"), "plain");
    }
}
