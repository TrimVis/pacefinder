//! Kodi NFO XML shapes for the three media kinds we care about:
//! `<tvshow>`, `<season>`, `<episodedetails>`.
//!
//! These types are intentionally close to the wire format. Conversion to
//! domain types lives in `From` impls so callers work with `model::*`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{Episode, NamedSeason, Season, Series};

// ---------- tvshow.nfo ----------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename = "tvshow")]
pub struct KodiTvShow {
    /// Jellyfin honors `<lockdata>true</lockdata>` by skipping provider
    /// refresh for this item, which stops the metadata explorer from
    /// silently rewriting our NFO on trivial interactions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lockdata: Option<bool>,
    pub title: String,
    pub showtitle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub originaltitle: Option<String>,
    pub plot: String,
    /// Kodi/Jellyfin honor `<displayorder>` to set the series-level
    /// episode ordering (e.g. "absolute" for a flat 1..N list, "aired"
    /// for per-season grouping, "dvd" for DVD order).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displayorder: Option<String>,
    #[serde(default, rename = "namedseason", skip_serializing_if = "Vec::is_empty")]
    pub namedseasons: Vec<KodiNamedSeason>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KodiNamedSeason {
    #[serde(rename = "@number")]
    pub number: u32,
    #[serde(rename = "$text")]
    pub name: String,
}

// ---------- season.nfo ----------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename = "season")]
pub struct KodiSeason {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lockdata: Option<bool>,
    pub title: String,
    pub seasonnumber: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot: Option<String>,
}

// ---------- episodedetails.nfo ----------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename = "episodedetails")]
pub struct KodiEpisode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lockdata: Option<bool>,
    pub title: String,
    pub showtitle: String,
    pub season: u32,
    pub episode: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub premiered: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aired: Option<String>,
}

// ---------- parsing ----------

pub fn parse_tvshow(xml: &str) -> Result<KodiTvShow> {
    quick_xml::de::from_str(xml).context("parsing tvshow.nfo")
}

pub fn parse_season(xml: &str) -> Result<KodiSeason> {
    quick_xml::de::from_str(xml).context("parsing season.nfo")
}

pub fn parse_episode(xml: &str) -> Result<KodiEpisode> {
    quick_xml::de::from_str(xml).context("parsing episode .nfo")
}

// ---------- conversions to domain ----------

impl From<KodiTvShow> for Series {
    fn from(k: KodiTvShow) -> Self {
        Series {
            title: k.title,
            showtitle: k.showtitle,
            original_title: k.originaltitle,
            plot: k.plot,
            display_order: k.displayorder,
            named_seasons: k
                .namedseasons
                .into_iter()
                .map(|ns| NamedSeason {
                    number: ns.number,
                    name: ns.name,
                })
                .collect(),
        }
    }
}

impl From<KodiSeason> for Season {
    fn from(k: KodiSeason) -> Self {
        Season {
            number: k.seasonnumber,
            title: k.title,
            plot: k.plot,
        }
    }
}

impl From<Series> for KodiTvShow {
    fn from(s: Series) -> Self {
        KodiTvShow {
            lockdata: None,
            title: s.title,
            showtitle: s.showtitle,
            originaltitle: s.original_title,
            plot: s.plot,
            displayorder: s.display_order,
            namedseasons: s
                .named_seasons
                .into_iter()
                .map(|ns| KodiNamedSeason {
                    number: ns.number,
                    name: ns.name,
                })
                .collect(),
        }
    }
}

impl From<Season> for KodiSeason {
    fn from(s: Season) -> Self {
        KodiSeason {
            lockdata: None,
            title: s.title,
            seasonnumber: s.number,
            plot: s.plot,
        }
    }
}

impl From<Episode> for KodiEpisode {
    fn from(e: Episode) -> Self {
        KodiEpisode {
            lockdata: None,
            title: e.title,
            showtitle: e.showtitle,
            season: e.season,
            episode: e.number,
            plot: e.plot,
            premiered: e.premiered,
            aired: e.aired,
        }
    }
}

impl From<KodiEpisode> for Episode {
    fn from(k: KodiEpisode) -> Self {
        Episode {
            showtitle: k.showtitle,
            season: k.season,
            number: k.episode,
            title: k.title,
            plot: k.plot,
            premiered: k.premiered,
            aired: k.aired,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TVSHOW_SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<tvshow>
  <title>One Pace</title>
  <showtitle>One Pace</showtitle>
  <originaltitle>One Piece</originaltitle>
  <plot>recut</plot>
  <namedseason number="1">1. Romance Dawn</namedseason>
  <namedseason number="2">2. Orange Town</namedseason>
</tvshow>"#;

    const SEASON_SAMPLE: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<season>
  <title>1. Romance Dawn</title>
  <seasonnumber>1</seasonnumber>
  <plot>Luffy sets out.</plot>
</season>"#;

    const EPISODE_SAMPLE: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<episodedetails>
  <title>Romance Dawn, the Dawn of an Adventure</title>
  <showtitle>One Pace</showtitle>
  <season>1</season>
  <episode>1</episode>
  <plot>Luffy meets Shanks.</plot>
  <premiered>2025-05-03</premiered>
  <aired>2025-05-03</aired>
</episodedetails>"#;

    #[test]
    fn parses_tvshow_with_named_seasons() {
        let k = parse_tvshow(TVSHOW_SAMPLE).unwrap();
        assert_eq!(k.title, "One Pace");
        assert_eq!(k.originaltitle.as_deref(), Some("One Piece"));
        assert_eq!(k.namedseasons.len(), 2);
        assert_eq!(k.namedseasons[0].number, 1);
        assert_eq!(k.namedseasons[0].name, "1. Romance Dawn");
        assert_eq!(k.namedseasons[1].number, 2);
    }

    #[test]
    fn parses_season() {
        let k = parse_season(SEASON_SAMPLE).unwrap();
        assert_eq!(k.seasonnumber, 1);
        assert_eq!(k.title, "1. Romance Dawn");
        assert_eq!(k.plot.as_deref(), Some("Luffy sets out."));
    }

    #[test]
    fn parses_episode() {
        let k = parse_episode(EPISODE_SAMPLE).unwrap();
        assert_eq!(k.title, "Romance Dawn, the Dawn of an Adventure");
        assert_eq!(k.season, 1);
        assert_eq!(k.episode, 1);
        assert_eq!(k.aired.as_deref(), Some("2025-05-03"));
    }

    #[test]
    fn tvshow_round_trips_to_domain() {
        let k = parse_tvshow(TVSHOW_SAMPLE).unwrap();
        let s: Series = k.into();
        assert_eq!(s.title, "One Pace");
        assert_eq!(s.named_seasons.len(), 2);
        assert_eq!(s.named_seasons[1].name, "2. Orange Town");
    }
}
