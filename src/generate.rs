//! `generate` subcommand: scan a One Pace library, fetch metadata,
//! and write Kodi-format NFO sidecars next to each video file.
//!
//! Two-phase: build a [`Vec<PendingWrite>`] from three planning functions,
//! classify each one against the existing filesystem, then apply per
//! `--force` / `--non-interactive` / interactive prompt.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use tracing::{info, warn};
use walkdir::WalkDir;

use crate::cli::LockMode;
use crate::fs_util::canonicalize_root;
use crate::matcher::{ParsedFile, is_arc_folder_name, normalize_arc};
use crate::nfo::writer::{self, MarkerStatus};
use crate::scan::is_video;
use crate::source::cache::CachedHttp;
use crate::source::{DataSource, ImageKind, default_chain};

const SAMPLE_EXPECTED_FILENAME: &str = "[One Pace][1] Romance Dawn 01 [1080p][D767799C].mkv";

pub struct Options {
    pub dry_run: bool,
    pub cache_ttl: Duration,
    pub refresh: bool,
    /// Overwrite conflicts (foreign files or files we wrote but the user
    /// has since edited) without asking.
    pub force: bool,
    /// Don't prompt — skip conflicts instead. Used by cron/CI. Mutually
    /// exclusive with `force`.
    pub non_interactive: bool,
    /// Value to write into the series' `<displayorder>` element (e.g.
    /// "absolute"). Overrides whatever the upstream Series carried.
    pub display_order: String,
    /// Which NFO kinds get `<lockdata>true</lockdata>` to stop Jellyfin's
    /// metadata explorer from rewriting them.
    pub lock: LockMode,
}

pub fn run(root: &Path, opts: Options) -> Result<()> {
    let root = canonicalize_root(root)?;
    info!(path = %root.display(), dry_run = opts.dry_run, "generating NFOs");

    warn_if_layout_looks_wrong(&root);

    let http = Rc::new(CachedHttp::new(opts.cache_ttl)?.refresh(opts.refresh));
    let source = default_chain(http);

    let scan = collect_matched(&root);
    info!(count = scan.matched.len(), "matched episode files");
    if scan.matched.is_empty() {
        report_empty_match(&root, &scan);
        return Ok(());
    }

    let lock_series = matches!(opts.lock, LockMode::Show | LockMode::All);
    let lock_children = matches!(opts.lock, LockMode::All);

    let mut pending: Vec<PendingWrite> = Vec::new();
    let mut episode_stats = EpisodeStats::default();
    plan_series_assets(
        source.as_ref(),
        &root,
        &opts.display_order,
        lock_series,
        &mut pending,
    );
    let arc_folders = plan_episode_assets(
        source.as_ref(),
        &root,
        &scan.matched,
        lock_children,
        &mut pending,
        &mut episode_stats,
    );
    plan_season_assets(source.as_ref(), &arc_folders, lock_children, &mut pending);

    let summary = apply_plan(pending, &opts)?;

    info!(
        episodes = episode_stats.matched_to_source,
        episode_fetch_failed = episode_stats.fetch_failed,
        unmatched = episode_stats.no_source_match,
        seasons = arc_folders.len(),
        wrote = summary.wrote,
        skipped = summary.skipped,
        unchanged = summary.unchanged,
        "done"
    );
    Ok(())
}

fn warn_if_layout_looks_wrong(root: &Path) {
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    if name.contains("one pace") {
        return;
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

// ---------- scan ----------

struct ScanReport {
    matched: Vec<(PathBuf, ParsedFile)>,
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

// ---------- write plan ----------

struct PendingWrite {
    path: PathBuf,
    label: String,
    kind: Asset,
}

enum Asset {
    SeriesNfo {
        series: crate::model::Series,
        lock: bool,
    },
    SeasonNfo {
        season: crate::model::Season,
        lock: bool,
    },
    EpisodeNfo {
        episode: crate::model::Episode,
        lock: bool,
    },
    Poster(Vec<u8>),
}

impl Asset {
    fn execute(self, path: &Path) -> Result<()> {
        match self {
            Self::SeriesNfo { series, lock } => writer::write_series(path, &series, lock),
            Self::SeasonNfo { season, lock } => writer::write_season(path, &season, lock),
            Self::EpisodeNfo { episode, lock } => writer::write_episode(path, &episode, lock),
            Self::Poster(bytes) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                fs::write(path, &bytes).with_context(|| format!("writing {}", path.display()))
            }
        }
    }
}

fn plan_series_assets(
    source: &dyn DataSource,
    root: &Path,
    display_order: &str,
    lock: bool,
    pending: &mut Vec<PendingWrite>,
) {
    match source.series() {
        Ok(Some(mut series)) => {
            series.display_order = Some(display_order.to_string());
            pending.push(PendingWrite {
                path: root.join("tvshow.nfo"),
                label: "tvshow.nfo".into(),
                kind: Asset::SeriesNfo { series, lock },
            });
            match source.image(ImageKind::SeriesPoster) {
                Ok(Some(bytes)) => pending.push(PendingWrite {
                    path: root.join("poster.png"),
                    label: "poster.png".into(),
                    kind: Asset::Poster(bytes),
                }),
                Ok(None) => {}
                Err(e) => warn!(error = %e, "fetching series poster failed"),
            }
        }
        Ok(None) => warn!("no series-level metadata from any data source"),
        Err(e) => {
            warn!(error = %e, "fetching series metadata failed; skipping series-level assets")
        }
    }
}

#[derive(Default)]
struct EpisodeStats {
    matched_to_source: usize,
    no_source_match: usize,
    fetch_failed: usize,
}

fn plan_episode_assets(
    source: &dyn DataSource,
    root: &Path,
    matched: &[(PathBuf, ParsedFile)],
    lock: bool,
    pending: &mut Vec<PendingWrite>,
    stats: &mut EpisodeStats,
) -> HashMap<u32, PathBuf> {
    let mut arc_folders: HashMap<u32, PathBuf> = HashMap::new();
    // Collapse regular + Extended pairs for the same (arc, episode):
    // Extended wins on disk. Demoted files get warned about (no NFO
    // written for them; user runs cleanup --remove-superseded to move
    // them aside).
    let winners = pick_winning_files(matched);
    let total = winners.len();

    for (i, (media_path, parsed)) in winners.iter().enumerate() {
        // Prefer CRC-based identification (Google Sheet) when the filename
        // has a CRC — that's the canonical (arc, episode) for the exact
        // file. Fall back to filename-derived arc + episode otherwise.
        let (arc_norm, episode_number) = parsed
            .crc32
            .as_deref()
            .and_then(|crc| source.identify_by_crc(crc).ok().flatten())
            .unwrap_or_else(|| (normalize_arc(&parsed.arc), parsed.episode));

        // Per-episode errors are logged and skipped, not propagated — a
        // flaky upstream shouldn't poison the whole library refresh.
        let mut episode = match source.episode(&arc_norm, episode_number) {
            Ok(Some(ep)) => ep,
            Ok(None) => {
                warn!(
                    file = %media_path.display(),
                    arc = %arc_norm,
                    episode = episode_number,
                    "no metadata found for this episode"
                );
                stats.no_source_match += 1;
                continue;
            }
            Err(e) => {
                warn!(
                    file = %media_path.display(),
                    arc = %arc_norm,
                    episode = episode_number,
                    error = %e,
                    "fetching episode metadata failed; skipping"
                );
                stats.fetch_failed += 1;
                continue;
            }
        };

        // Surface the variant in the title so users see "Arlong Park 5
        // (Extended)" in Jellyfin instead of just "Arlong Park 5".
        if parsed.extended {
            episode.title = format!("{} (Extended)", episode.title);
        }

        let nfo_path = media_path.with_extension("nfo");
        let label = format!(
            "S{season:02}E{number:02} ({i}/{total})",
            season = episode.season,
            number = episode.number,
            i = i + 1,
        );
        if let Some(parent) = media_path.parent()
            && parent != root
        {
            arc_folders
                .entry(episode.season)
                .or_insert_with(|| parent.to_path_buf());
        }
        pending.push(PendingWrite {
            path: nfo_path,
            label,
            kind: Asset::EpisodeNfo { episode, lock },
        });
        stats.matched_to_source += 1;
    }

    arc_folders
}

/// For each (arc, episode) slot, pick the one file whose NFO we'll
/// actually emit. Extended wins when both a regular and Extended cut
/// exist; warns on the discarded file so the user can act.
fn pick_winning_files(matched: &[(PathBuf, ParsedFile)]) -> Vec<&(PathBuf, ParsedFile)> {
    use std::collections::hash_map::Entry;
    let mut slots: HashMap<(String, u32), &(PathBuf, ParsedFile)> = HashMap::new();
    for entry in matched {
        let key = (normalize_arc(&entry.1.arc), entry.1.episode);
        match slots.entry(key) {
            Entry::Vacant(v) => {
                v.insert(entry);
            }
            Entry::Occupied(mut o) => {
                let current = *o.get();
                if entry.1.extended && !current.1.extended {
                    warn!(
                        superseded = %current.0.display(),
                        kept = %entry.0.display(),
                        "regular cut superseded by Extended — run `pacefinder cleanup --remove-superseded` to move the regular aside",
                    );
                    o.insert(entry);
                } else if !entry.1.extended && current.1.extended {
                    warn!(
                        superseded = %entry.0.display(),
                        kept = %current.0.display(),
                        "regular cut superseded by Extended — run `pacefinder cleanup --remove-superseded` to move the regular aside",
                    );
                } else {
                    warn!(
                        kept = %current.0.display(),
                        duplicate = %entry.0.display(),
                        "two files for the same (arc, episode) with the same variant — keeping first",
                    );
                }
            }
        }
    }
    slots.into_values().collect()
}

fn plan_season_assets(
    source: &dyn DataSource,
    arc_folders: &HashMap<u32, PathBuf>,
    lock: bool,
    pending: &mut Vec<PendingWrite>,
) {
    for (season_num, folder) in arc_folders {
        match source.season(*season_num) {
            Ok(Some(season)) => pending.push(PendingWrite {
                path: folder.join("season.nfo"),
                label: format!("season.nfo (S{season_num:02})"),
                kind: Asset::SeasonNfo { season, lock },
            }),
            Ok(None) => warn!(season = season_num, "no season metadata available"),
            Err(e) => {
                warn!(season = season_num, error = %e, "fetching season metadata failed");
                continue;
            }
        }
        match source.image(ImageKind::SeasonPoster {
            number: *season_num,
        }) {
            Ok(Some(bytes)) => pending.push(PendingWrite {
                path: folder.join("poster.png"),
                label: format!("poster.png (S{season_num:02})"),
                kind: Asset::Poster(bytes),
            }),
            Ok(None) => {}
            Err(e) => warn!(season = season_num, error = %e, "fetching season poster failed"),
        }
    }
}

// ---------- apply ----------

#[derive(Debug)]
enum WriteStatus {
    /// Path doesn't exist. Safe.
    Fresh,
    /// NFO marked by us and unchanged since. Safe.
    UpdateOurs,
    /// NFO marked by us but the user has edited it since. Conflict.
    UserEdited,
    /// NFO present but doesn't carry our marker — foreign tool or hand-written. Conflict.
    ForeignNfo,
    /// Poster bytes on disk match what we'd write. No-op.
    PosterUnchanged,
    /// Poster present, bytes differ from what we'd write. Conflict.
    PosterDiffers,
}

fn classify(p: &PendingWrite) -> WriteStatus {
    if let Asset::Poster(new_bytes) = &p.kind {
        if !p.path.exists() {
            return WriteStatus::Fresh;
        }
        let same = match fs::read(&p.path) {
            Ok(existing) => bytes_sha256(&existing) == bytes_sha256(new_bytes),
            Err(_) => false,
        };
        return if same {
            WriteStatus::PosterUnchanged
        } else {
            WriteStatus::PosterDiffers
        };
    }
    if !p.path.exists() {
        return WriteStatus::Fresh;
    }
    match writer::marker_status(&p.path) {
        MarkerStatus::IntactOurs => WriteStatus::UpdateOurs,
        MarkerStatus::EditedOurs => WriteStatus::UserEdited,
        MarkerStatus::Absent => WriteStatus::ForeignNfo,
    }
}

fn bytes_sha256(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

struct ApplySummary {
    wrote: usize,
    skipped: usize,
    unchanged: usize,
}

fn apply_plan(pending: Vec<PendingWrite>, opts: &Options) -> Result<ApplySummary> {
    // Bucket writes per classification.
    let mut safe: Vec<PendingWrite> = Vec::new();
    let mut conflicts: Vec<(PendingWrite, WriteStatus)> = Vec::new();
    let mut unchanged = 0usize;
    for write in pending {
        match classify(&write) {
            WriteStatus::Fresh | WriteStatus::UpdateOurs => safe.push(write),
            WriteStatus::PosterUnchanged => unchanged += 1,
            status @ (WriteStatus::UserEdited
            | WriteStatus::ForeignNfo
            | WriteStatus::PosterDiffers) => conflicts.push((write, status)),
        }
    }

    let mut summary = ApplySummary {
        wrote: 0,
        skipped: 0,
        unchanged,
    };

    for write in safe {
        execute_one(write, opts.dry_run, &mut summary)?;
    }

    if conflicts.is_empty() {
        return Ok(summary);
    }
    resolve_conflicts(conflicts, opts, &mut summary)
}

fn resolve_conflicts(
    conflicts: Vec<(PendingWrite, WriteStatus)>,
    opts: &Options,
    summary: &mut ApplySummary,
) -> Result<ApplySummary> {
    if opts.dry_run {
        warn!(
            "[dry-run] {} item(s) would be skipped or need a decision (foreign / user-edited / changed poster). \
             Re-run with --force to overwrite or --non-interactive to skip them.",
            conflicts.len()
        );
        for (c, status) in &conflicts {
            info!(would_conflict = %c.path.display(), reason = ?status, "[dry-run] {}", c.label);
        }
        summary.skipped += conflicts.len();
        return Ok(ApplySummary {
            wrote: summary.wrote,
            skipped: summary.skipped,
            unchanged: summary.unchanged,
        });
    }
    if opts.force {
        info!(
            "--force: overwriting {} conflicting file(s)",
            conflicts.len()
        );
        for (w, _) in conflicts {
            execute_one(w, false, summary)?;
        }
        return Ok(ApplySummary {
            wrote: summary.wrote,
            skipped: summary.skipped,
            unchanged: summary.unchanged,
        });
    }
    if opts.non_interactive {
        warn!(
            "--non-interactive: skipping {} conflict(s); re-run with --force to overwrite",
            conflicts.len()
        );
        for (c, _) in &conflicts {
            warn!(skipped = %c.path.display(), "  {}", c.label);
        }
        summary.skipped += conflicts.len();
        return Ok(ApplySummary {
            wrote: summary.wrote,
            skipped: summary.skipped,
            unchanged: summary.unchanged,
        });
    }

    // Interactive: require a TTY. Silently skipping when piped to /dev/null
    // is the worst outcome — the user wouldn't know conflicts existed.
    if !io::stdin().is_terminal() {
        bail!(
            "{} write(s) would clobber existing files but stdin isn't a terminal — \
             pass --force (overwrite), --non-interactive (skip), or --dry-run (preview)",
            conflicts.len()
        );
    }

    println!();
    println!(
        "{} existing file(s) would be overwritten and need confirmation:",
        conflicts.len()
    );
    let preview_count = conflicts.len().min(10);
    for (c, status) in conflicts.iter().take(preview_count) {
        println!("  [{:?}] {}", status, c.path.display());
    }
    if conflicts.len() > preview_count {
        println!("  ... and {} more", conflicts.len() - preview_count);
    }
    print!("Overwrite all? [y/N] ");
    io::stdout().flush().ok();
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    let answer = answer.trim().to_ascii_lowercase();
    if answer == "y" || answer == "yes" {
        for (w, _) in conflicts {
            execute_one(w, false, summary)?;
        }
    } else {
        warn!(
            "skipped {} conflicting write(s); re-run with --force to overwrite without prompting",
            conflicts.len()
        );
        summary.skipped += conflicts.len();
    }
    Ok(ApplySummary {
        wrote: summary.wrote,
        skipped: summary.skipped,
        unchanged: summary.unchanged,
    })
}

fn execute_one(write: PendingWrite, dry_run: bool, summary: &mut ApplySummary) -> Result<()> {
    let bytes_hint = match &write.kind {
        Asset::Poster(b) => Some(b.len()),
        _ => None,
    };
    if dry_run {
        match bytes_hint {
            Some(n) => {
                info!(would_write = %write.path.display(), bytes = n, "[dry-run] {}", write.label)
            }
            None => info!(would_write = %write.path.display(), "[dry-run] {}", write.label),
        }
        summary.wrote += 1;
        return Ok(());
    }
    let path = write.path.clone();
    let label = write.label.clone();
    write.kind.execute(&path)?;
    match bytes_hint {
        Some(n) => info!(path = %path.display(), bytes = n, "wrote {label}"),
        None => info!(path = %path.display(), "wrote {label}"),
    }
    summary.wrote += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, arc: &str, ep: u32, extended: bool) -> (PathBuf, ParsedFile) {
        (
            PathBuf::from(path),
            ParsedFile {
                arc: arc.into(),
                episode: ep,
                crc32: None,
                resolution: Some("1080p".into()),
                extended,
            },
        )
    }

    #[test]
    fn pick_winning_files_extended_supersedes_regular() {
        let matched = vec![
            entry("/lib/arc/regular.mkv", "Arlong Park", 5, false),
            entry("/lib/arc/extended.mkv", "Arlong Park", 5, true),
        ];
        let winners = pick_winning_files(&matched);
        assert_eq!(winners.len(), 1);
        assert!(winners[0].1.extended);
    }

    #[test]
    fn pick_winning_files_keeps_unpaired_regular() {
        let matched = vec![entry("/lib/arc/r.mkv", "Wano", 1, false)];
        let winners = pick_winning_files(&matched);
        assert_eq!(winners.len(), 1);
        assert!(!winners[0].1.extended);
    }

    #[test]
    fn pick_winning_files_distinct_episodes_independent() {
        let matched = vec![
            entry("/lib/wano/r1.mkv", "Wano", 1, false),
            entry("/lib/wano/r2.mkv", "Wano", 2, false),
        ];
        let winners = pick_winning_files(&matched);
        assert_eq!(winners.len(), 2);
    }

    #[test]
    fn pick_winning_files_pairs_via_normalized_arc() {
        // Different folder casing → same normalized arc → pair.
        let matched = vec![
            entry("/lib/x/r.mkv", "arlong park", 5, false),
            entry("/lib/y/e.mkv", "Arlong Park", 5, true),
        ];
        let winners = pick_winning_files(&matched);
        assert_eq!(winners.len(), 1);
        assert!(winners[0].1.extended);
    }
}
