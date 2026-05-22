//! `generate` subcommand: scan a One Pace library, fetch metadata,
//! and write Kodi-format NFO sidecars next to each video file.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use walkdir::WalkDir;

use tokio::fs;

use crate::matcher::{ParsedFile, normalize_arc};
use crate::model::ImageKind;
use crate::nfo::writer;
use crate::source::DataSource;
use crate::source::cache::CachedHttp;
use crate::source::composite::Composite;
use crate::source::onepacenet::OnepaceNet;
use crate::source::spykernz::SpykerNz;

const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "m4v", "avi"];

pub struct Options {
    pub dry_run: bool,
    pub cache_ttl: Duration,
    pub refresh: bool,
}

pub async fn run(root: &Path, opts: Options) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;
    info!(path = %root.display(), dry_run = opts.dry_run, "generating NFOs");

    let http = Arc::new(CachedHttp::new(opts.cache_ttl)?.refresh(opts.refresh));
    // Order: onepace.net first (current arc list + fresh descriptions),
    // SpykerNZ second (episodes, posters, series-level fallback).
    let source: Arc<dyn DataSource> = Arc::new(Composite::new(vec![
        Arc::new(OnepaceNet::new(http.clone())),
        Arc::new(SpykerNz::new(http)),
    ]));

    let matched = collect_matched(&root);
    info!(count = matched.len(), "matched episode files");
    if matched.is_empty() {
        warn!("no One Pace files matched — nothing to do");
        return Ok(());
    }

    if let Some(series) = source.series().await.context("fetching series metadata")? {
        let series_path = root.join("tvshow.nfo");
        write(opts.dry_run, &series_path, "tvshow.nfo", || async {
            writer::write_series(&series_path, &series).await
        })
        .await?;

        let series_poster_path = root.join("poster.png");
        fetch_image(
            opts.dry_run,
            source.as_ref(),
            ImageKind::SeriesPoster,
            &series_poster_path,
            "poster.png",
        )
        .await?;
    } else {
        warn!("no series-level metadata from any data source");
    }

    let mut arc_folders: HashMap<u32, PathBuf> = HashMap::new();
    let mut episodes_written = 0usize;
    let mut episodes_unmatched = 0usize;

    for (media_path, parsed) in &matched {
        let arc_norm = normalize_arc(&parsed.arc);
        let Some(episode) = source
            .episode(&arc_norm, parsed.episode)
            .await
            .with_context(|| format!("fetching episode for {}", media_path.display()))?
        else {
            warn!(
                file = %media_path.display(),
                arc = %parsed.arc,
                episode = parsed.episode,
                "no metadata found for this episode"
            );
            episodes_unmatched += 1;
            continue;
        };

        let nfo_path = media_path.with_extension("nfo");
        let label = format!("S{:02}E{:02}", episode.season, episode.number);
        write(opts.dry_run, &nfo_path, &label, || async {
            writer::write_episode(&nfo_path, &episode).await
        })
        .await?;
        episodes_written += 1;

        if let Some(parent) = media_path.parent() {
            if parent != root {
                arc_folders
                    .entry(episode.season)
                    .or_insert_with(|| parent.to_path_buf());
            }
        }
    }

    for (season_num, folder) in &arc_folders {
        let Some(season) = source
            .season(*season_num)
            .await
            .with_context(|| format!("fetching season {season_num}"))?
        else {
            warn!(season = season_num, "no season metadata available");
            continue;
        };
        let nfo_path = folder.join("season.nfo");
        let label = format!("season.nfo (S{:02})", season_num);
        write(opts.dry_run, &nfo_path, &label, || async {
            writer::write_season(&nfo_path, &season).await
        })
        .await?;

        let poster_path = folder.join("poster.png");
        let label = format!("poster.png (S{:02})", season_num);
        fetch_image(
            opts.dry_run,
            source.as_ref(),
            ImageKind::SeasonPoster { number: *season_num },
            &poster_path,
            &label,
        )
        .await?;
    }

    info!(
        episodes = episodes_written,
        unmatched = episodes_unmatched,
        seasons = arc_folders.len(),
        "done"
    );
    Ok(())
}

fn collect_matched(root: &Path) -> Vec<(PathBuf, ParsedFile)> {
    let mut out = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.into_path();
        if !is_video(&path) {
            continue;
        }
        match ParsedFile::from_path(&path) {
            Some(parsed) => out.push((path, parsed)),
            None => warn!(file = %path.display(), "filename does not look like a One Pace release"),
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            VIDEO_EXTS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

async fn fetch_image(
    dry_run: bool,
    source: &dyn DataSource,
    kind: ImageKind,
    path: &Path,
    label: &str,
) -> Result<()> {
    let Some(bytes) = source
        .image(kind)
        .await
        .with_context(|| format!("fetching {label}"))?
    else {
        warn!(image = %label, "no image available from source");
        return Ok(());
    };
    if dry_run {
        info!(would_write = %path.display(), bytes = bytes.len(), "[dry-run] {label}");
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, &bytes)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    info!(path = %path.display(), bytes = bytes.len(), "wrote {label}");
    Ok(())
}

async fn write<F, Fut>(dry_run: bool, path: &Path, label: &str, op: F) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    if dry_run {
        info!(would_write = %path.display(), "[dry-run] {label}");
        Ok(())
    } else {
        op().await
    }
}
