//! `generate` subcommand: scan a One Pace library, fetch metadata,
//! and write Kodi-format NFO sidecars next to each video file.

use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use walkdir::WalkDir;

use crate::matcher::{ParsedFile, is_arc_folder_name, normalize_arc};
use crate::nfo::writer;
use crate::scan::is_video;
use crate::source::cache::CachedHttp;
use crate::source::composite::Composite;
use crate::source::onepacenet::OnepaceNet;
use crate::source::spykernz::SpykerNz;
use crate::source::{DataSource, ImageKind};

const SAMPLE_EXPECTED_FILENAME: &str = "[One Pace][1] Romance Dawn 01 [1080p][D767799C].mkv";

pub struct Options {
    pub dry_run: bool,
    pub cache_ttl: Duration,
    pub refresh: bool,
}

pub fn run(root: &Path, opts: Options) -> Result<()> {
    let root = canonicalize_or_helpful_error(root)?;
    info!(path = %root.display(), dry_run = opts.dry_run, "generating NFOs");

    warn_if_layout_looks_wrong(&root);

    let source = build_source(opts.cache_ttl, opts.refresh)?;

    let scan = collect_matched(&root);
    info!(count = scan.matched.len(), "matched episode files");
    if scan.matched.is_empty() {
        report_empty_match(&root, &scan);
        return Ok(());
    }

    write_series_assets(source.as_ref(), &root, opts.dry_run)?;
    let report = write_episode_assets(source.as_ref(), &root, &scan.matched, opts.dry_run)?;
    write_season_assets(source.as_ref(), &report.arc_folders, opts.dry_run)?;

    info!(
        episodes = report.written,
        unmatched = report.unmatched,
        seasons = report.arc_folders.len(),
        "done"
    );
    Ok(())
}

fn canonicalize_or_helpful_error(root: &Path) -> Result<PathBuf> {
    root.canonicalize().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            anyhow!("path does not exist: {}", root.display())
        } else {
            anyhow!("{}: {}", root.display(), e)
        }
    })
}

/// If `root` looks like a library root (arc folders directly inside, name
/// doesn't mention "one pace"), warn — we'd happily write `tvshow.nfo`
/// there and Jellyfin would treat every arc as its own Series.
fn warn_if_layout_looks_wrong(root: &Path) {
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    if name.contains("one pace") {
        return; // looks like a series folder
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let arc_count = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| is_arc_folder_name(n))
        .count();
    if arc_count >= 2 {
        warn!(
            "path {} doesn't look like a One Pace series folder (its name doesn't \
             contain 'one pace') but {} arc folders sit directly inside it. \
             If your layout is <library>/<arc>/<episode>, Jellyfin will treat each \
             arc as a separate Series. Consider running: pacefinder reorder {}",
            root.display(),
            arc_count,
            root.display()
        );
    }
}

fn build_source(cache_ttl: Duration, refresh: bool) -> Result<Arc<dyn DataSource>> {
    let http = Arc::new(CachedHttp::new(cache_ttl)?.refresh(refresh));
    // Order: onepace.net first (current arc list + fresh descriptions),
    // SpykerNZ second (episodes, posters, series-level fallback).
    Ok(Arc::new(Composite::new(vec![
        Arc::new(OnepaceNet::new(http.clone())),
        Arc::new(SpykerNz::new(http)),
    ])))
}

fn write_series_assets(source: &dyn DataSource, root: &Path, dry_run: bool) -> Result<()> {
    let Some(series) = source.series().context("fetching series metadata")? else {
        warn!("no series-level metadata from any data source");
        return Ok(());
    };
    let series_path = root.join("tvshow.nfo");
    write(dry_run, &series_path, "tvshow.nfo", || {
        writer::write_series(&series_path, &series)
    })?;
    let series_poster_path = root.join("poster.png");
    fetch_image(
        dry_run,
        source,
        ImageKind::SeriesPoster,
        &series_poster_path,
        "poster.png",
    )
}

struct EpisodeReport {
    written: usize,
    unmatched: usize,
    /// Maps each season number we saw an episode for back to the folder
    /// that episode lives in. Consumed by `write_season_assets`.
    arc_folders: HashMap<u32, PathBuf>,
}

fn write_episode_assets(
    source: &dyn DataSource,
    root: &Path,
    matched: &[(PathBuf, ParsedFile)],
    dry_run: bool,
) -> Result<EpisodeReport> {
    let mut arc_folders: HashMap<u32, PathBuf> = HashMap::new();
    let mut written = 0usize;
    let mut unmatched = 0usize;
    let total = matched.len();

    for (i, (media_path, parsed)) in matched.iter().enumerate() {
        let arc_norm = normalize_arc(&parsed.arc);
        let Some(episode) = source
            .episode(&arc_norm, parsed.episode)
            .with_context(|| format!("fetching episode for {}", media_path.display()))?
        else {
            warn!(
                file = %media_path.display(),
                arc = %parsed.arc,
                episode = parsed.episode,
                "no metadata found for this episode"
            );
            unmatched += 1;
            continue;
        };

        let nfo_path = media_path.with_extension("nfo");
        let label = format!(
            "S{season:02}E{number:02} ({i}/{total})",
            season = episode.season,
            number = episode.number,
            i = i + 1,
        );
        write(dry_run, &nfo_path, &label, || {
            writer::write_episode(&nfo_path, &episode)
        })?;
        written += 1;

        if let Some(parent) = media_path.parent()
            && parent != root
        {
            arc_folders
                .entry(episode.season)
                .or_insert_with(|| parent.to_path_buf());
        }
    }

    Ok(EpisodeReport {
        written,
        unmatched,
        arc_folders,
    })
}

fn write_season_assets(
    source: &dyn DataSource,
    arc_folders: &HashMap<u32, PathBuf>,
    dry_run: bool,
) -> Result<()> {
    for (season_num, folder) in arc_folders {
        let Some(season) = source
            .season(*season_num)
            .with_context(|| format!("fetching season {season_num}"))?
        else {
            warn!(season = season_num, "no season metadata available");
            continue;
        };
        let nfo_path = folder.join("season.nfo");
        let label = format!("season.nfo (S{season_num:02})");
        write(dry_run, &nfo_path, &label, || {
            writer::write_season(&nfo_path, &season)
        })?;

        let poster_path = folder.join("poster.png");
        let label = format!("poster.png (S{season_num:02})");
        fetch_image(
            dry_run,
            source,
            ImageKind::SeasonPoster {
                number: *season_num,
            },
            &poster_path,
            &label,
        )?;
    }
    Ok(())
}

struct ScanReport {
    matched: Vec<(PathBuf, ParsedFile)>,
    /// Total `.mkv`/`.mp4`/etc files seen, regardless of whether their name
    /// looked like a One Pace release.
    total_videos: usize,
}

fn collect_matched(root: &Path) -> ScanReport {
    let mut matched = Vec::new();
    let mut total_videos = 0usize;
    for entry in WalkDir::new(root).follow_links(false) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.into_path();
        if !is_video(&path) {
            continue;
        }
        total_videos += 1;
        if let Some(parsed) = ParsedFile::from_path(&path) {
            matched.push((path, parsed));
        } else {
            warn!(file = %path.display(), "filename does not look like a One Pace release");
        }
    }
    matched.sort_by(|a, b| a.0.cmp(&b.0));
    ScanReport {
        matched,
        total_videos,
    }
}

fn report_empty_match(root: &Path, scan: &ScanReport) {
    if scan.total_videos == 0 {
        warn!("no video files found under {}", root.display());
    } else {
        warn!(
            "found {} video files under {} but none matched the One Pace naming scheme. \
             Expected filenames like: {}",
            scan.total_videos,
            root.display(),
            SAMPLE_EXPECTED_FILENAME,
        );
    }
}

fn fetch_image(
    dry_run: bool,
    source: &dyn DataSource,
    kind: ImageKind,
    path: &Path,
    label: &str,
) -> Result<()> {
    let Some(bytes) = source
        .image(kind)
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
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, &bytes).with_context(|| format!("writing {}", path.display()))?;
    info!(path = %path.display(), bytes = bytes.len(), "wrote {label}");
    Ok(())
}

fn write<F>(dry_run: bool, path: &Path, label: &str, op: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    if dry_run {
        info!(would_write = %path.display(), "[dry-run] {label}");
        Ok(())
    } else {
        op()
    }
}
