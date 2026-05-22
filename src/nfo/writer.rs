//! Emit Kodi-format NFO files next to user media.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use tracing::debug;

use super::kodi::{KodiEpisode, KodiSeason, KodiTvShow};
use crate::model::{Episode, Season, Series};

const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

pub fn write_series(path: &Path, series: &Series) -> Result<()> {
    let kodi: KodiTvShow = series.clone().into();
    write_xml(path, &kodi, "tvshow")
}

pub fn write_season(path: &Path, season: &Season) -> Result<()> {
    let kodi: KodiSeason = season.clone().into();
    write_xml(path, &kodi, "season")
}

pub fn write_episode(path: &Path, episode: &Episode) -> Result<()> {
    let kodi: KodiEpisode = episode.clone().into();
    write_xml(path, &kodi, "episodedetails")
}

fn write_xml<T: serde::Serialize>(path: &Path, value: &T, root: &str) -> Result<()> {
    let body = quick_xml::se::to_string_with_root(root, value)
        .with_context(|| format!("serializing <{root}>"))?;
    let out = format!("{XML_DECL}{body}\n");

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
    debug!(path = %path.display(), "wrote nfo");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Episode, NamedSeason, Season, Series};
    use crate::nfo::kodi;
    use tempfile::tempdir;

    fn sample_series() -> Series {
        Series {
            title: "One Pace".into(),
            showtitle: "One Pace".into(),
            original_title: Some("One Piece".into()),
            plot: "Fan recut.".into(),
            named_seasons: vec![
                NamedSeason {
                    number: 1,
                    name: "1. Romance Dawn".into(),
                },
                NamedSeason {
                    number: 2,
                    name: "2. Orange Town".into(),
                },
            ],
        }
    }

    #[test]
    fn series_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tvshow.nfo");
        let s = sample_series();
        write_series(&path, &s).unwrap();

        let xml = fs::read_to_string(&path).unwrap();
        let parsed: Series = kodi::parse_tvshow(&xml).unwrap().into();
        assert_eq!(parsed.title, s.title);
        assert_eq!(parsed.original_title, s.original_title);
        assert_eq!(parsed.named_seasons, s.named_seasons);
    }

    #[test]
    fn season_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("season.nfo");
        let s = Season {
            number: 1,
            title: "1. Romance Dawn".into(),
            plot: Some("Luffy sets out.".into()),
        };
        write_season(&path, &s).unwrap();
        let xml = fs::read_to_string(&path).unwrap();
        let parsed: Season = kodi::parse_season(&xml).unwrap().into();
        assert_eq!(parsed.number, 1);
        assert_eq!(parsed.title, s.title);
    }

    #[test]
    fn episode_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ep.nfo");
        let e = Episode {
            showtitle: "One Pace".into(),
            season: 1,
            number: 1,
            title: "Romance Dawn, the Dawn of an Adventure".into(),
            plot: Some("Luffy meets Shanks.".into()),
            premiered: Some("2025-05-03".into()),
            aired: Some("2025-05-03".into()),
        };
        write_episode(&path, &e).unwrap();
        let xml = fs::read_to_string(&path).unwrap();
        let parsed: Episode = kodi::parse_episode(&xml).unwrap().into();
        assert_eq!(parsed.title, e.title);
        assert_eq!(parsed.season, 1);
        assert_eq!(parsed.number, 1);
        assert_eq!(parsed.aired, e.aired);
    }
}
