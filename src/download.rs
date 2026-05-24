//! `download` subcommand: queue missing One Pace releases for download.
//!
//! Flow:
//!
//! 1. Scrape onepace.net `/releases` for every available magnet URI.
//! 2. Filter by resolution and (optionally) arc-name substring.
//! 3. Group per (arc, episode); pick the highest-resolution release that
//!    still fits under the user's `--resolution` cap.
//! 4. Build "have" set from the library (CRC32 of every recognized .mkv).
//! 5. Build "queued" set from qBittorrent's current torrent names.
//! 6. Queue any release whose CRC isn't in either set, with `save_path`
//!    pointing at the arc folder under `<series-root>`.
//! 7. (Optional) `--prepopulate-nfo`: write episode.nfo at the future
//!    target path so Jellyfin's first scan after download has metadata.
//!
//! Queue-and-go: no waiting, no progress reporting. Compose with
//! `pacefinder generate` after downloads finish (or use `--prepopulate-nfo`
//! to get most of the way there before the .mkv lands).

use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::dl::Release;
use crate::dl::qbittorrent::QbtClient;
use crate::matcher::{ParsedFile, normalize_arc};
use crate::nfo::writer;
use crate::scan::is_video;
use crate::source::DataSource;
use crate::source::cache::CachedHttp;
use crate::source::composite::Composite;
use crate::source::onepacenet::OnepaceNet;
use crate::source::sheet::GoogleSheet;
use crate::source::spykernz::SpykerNz;

pub struct Options {
    pub qbt_url: String,
    pub qbt_user: String,
    pub qbt_pass: String,
    pub qbt_category: Option<String>,
    pub resolution: String,
    pub cache_ttl: Duration,
    pub refresh: bool,
    pub dry_run: bool,
    pub prepopulate_nfo: bool,
    pub refresh_existing: bool,
    pub only_arc: Option<String>,
    /// `HOST=CONTAINER` — translate save paths from pacefinder's view to
    /// qBittorrent's view when they differ (Docker mount tables etc.).
    pub save_path_map: Option<String>,
}

/// Parsed `--save-path-map`. Trailing slashes on each side are tolerated.
#[derive(Debug)]
struct PathMap {
    host: PathBuf,
    container: PathBuf,
}

impl PathMap {
    fn parse(s: &str) -> Result<Self> {
        let (host, container) = s.split_once('=').ok_or_else(|| {
            anyhow!(
                "--save-path-map must be HOST=CONTAINER (e.g. /mnt/media=/downloads), got {s:?}"
            )
        })?;
        let host = host.trim_end_matches('/');
        let container = container.trim_end_matches('/');
        if host.is_empty() || container.is_empty() {
            bail!("--save-path-map sides must be non-empty: {s:?}");
        }
        // Canonicalize the host side so symlinked library roots still match
        // the prefix check. Falls back to the lexical path if the host
        // prefix doesn't exist locally (cross-machine setups).
        let host_path = PathBuf::from(host);
        let host_canon = host_path.canonicalize().unwrap_or(host_path);
        Ok(Self {
            host: host_canon,
            container: PathBuf::from(container),
        })
    }

    /// Map a host-side path into the container's filesystem view. Returns
    /// `None` if the path doesn't sit under the configured host prefix
    /// (caller treats this as a setup mistake).
    fn translate(&self, host_path: &Path) -> Option<PathBuf> {
        host_path
            .strip_prefix(&self.host)
            .ok()
            .map(|tail| self.container.join(tail))
    }
}

pub fn run(root: &Path, opts: Options) -> Result<()> {
    let root = canonicalize_or_helpful_error(root)?;
    info!(
        path = %root.display(),
        resolution = %opts.resolution,
        dry_run = opts.dry_run,
        prepopulate_nfo = opts.prepopulate_nfo,
        "queueing missing releases",
    );

    let max_height = parse_resolution_cap(&opts.resolution)?;
    let http = Rc::new(CachedHttp::new(opts.cache_ttl)?.refresh(opts.refresh));

    // Parse and validate the save-path map early — fail fast on typos.
    let path_map = opts
        .save_path_map
        .as_deref()
        .map(PathMap::parse)
        .transpose()?;
    if let Some(m) = &path_map
        && !root.starts_with(&m.host)
    {
        bail!(
            "--save-path-map host prefix {} doesn't apply to library root {} \
             (the prefix must be a parent of the library path)",
            m.host.display(),
            root.display(),
        );
    }

    // 1. Fetch releases.
    let onepacenet = OnepaceNet::new(http.clone());
    let releases = onepacenet
        .fetch_releases()
        .context("fetching releases from onepace.net")?;
    info!(count = releases.len(), "release listings discovered");

    // 2 + 3. Filter by resolution + only_arc; collapse to best-per-episode.
    let chosen = pick_best_per_episode(releases, max_height, opts.only_arc.as_deref());
    info!(
        count = chosen.len(),
        "candidate releases after resolution/arc filter"
    );
    if chosen.is_empty() {
        warn!("nothing to consider — adjust --resolution or --only-arc");
        return Ok(());
    }

    // 4. Library CRCs.
    let have = library_crcs(&root);
    debug!(count = have.len(), "library file CRCs");

    // 5. qBittorrent state (skip when dry-run; we don't want to require
    //    credentials just to preview).
    let (qbt, queued) = if opts.dry_run {
        (None, std::collections::HashSet::new())
    } else {
        let client = QbtClient::login(&opts.qbt_url, &opts.qbt_user, &opts.qbt_pass)
            .context("logging in to qBittorrent")?;
        // Best-effort hint when our save paths probably don't make sense
        // from qBittorrent's perspective. False positives possible; we
        // only nudge, never block.
        warn_if_save_path_likely_unreachable(&client, &root, path_map.as_ref());
        let names = client
            .list_torrent_names()
            .context("listing current qBittorrent torrents")?;
        let queued: std::collections::HashSet<String> = names
            .iter()
            .filter_map(|n| ParsedFile::from_filename(n).and_then(|p| p.crc32))
            .collect();
        debug!(
            count = queued.len(),
            "CRCs of torrents already in qBittorrent"
        );
        (Some(client), queued)
    };

    // 6 + 7. Queue each missing release; optionally pre-write NFO.
    let source = if opts.prepopulate_nfo {
        Some(build_metadata_source(http.clone()))
    } else {
        None
    };

    let mut stats = RunStats::default();
    for release in chosen {
        let Some(parsed) = release.parsed.as_ref() else {
            stats.unparseable += 1;
            continue;
        };
        let Some(crc) = parsed.crc32.as_deref() else {
            warn!(file = %release.filename, "release filename has no CRC; skipping");
            stats.no_crc += 1;
            continue;
        };
        if !opts.refresh_existing && have.contains(crc) {
            stats.already_have += 1;
            continue;
        }
        if queued.contains(crc) {
            stats.already_queued += 1;
            continue;
        }

        // Figure out the arc folder we'd want the torrent to land in.
        let arc_folder = find_or_propose_arc_folder(&root, &parsed.arc);
        let save_path_host = root.join(&arc_folder);
        // Translate to qBittorrent's view if a mapping was configured.
        // `validate_*` above guarantees the prefix matches.
        let save_path = match &path_map {
            Some(m) => m.translate(&save_path_host).ok_or_else(|| {
                anyhow!(
                    "save path {} fell outside host prefix {} (path map bug?)",
                    save_path_host.display(),
                    m.host.display(),
                )
            })?,
            None => save_path_host.clone(),
        };

        if opts.dry_run {
            if path_map.is_some() {
                info!(
                    magnet = %short_magnet(&release.magnet),
                    save_path = %save_path.display(),
                    host_path = %save_path_host.display(),
                    file = %release.filename,
                    "[dry-run] would queue",
                );
            } else {
                info!(
                    magnet = %short_magnet(&release.magnet),
                    save_path = %save_path.display(),
                    file = %release.filename,
                    "[dry-run] would queue",
                );
            }
        } else if let Some(qbt) = qbt.as_ref() {
            if let Err(e) =
                qbt.add_magnet(&release.magnet, &save_path, opts.qbt_category.as_deref())
            {
                warn!(file = %release.filename, error = %e, "queue failed; continuing");
                stats.queue_failed += 1;
                continue;
            }
            info!(file = %release.filename, save_path = %save_path.display(), "queued");
        }
        stats.queued += 1;

        if opts.prepopulate_nfo
            && let Some(src) = source.as_ref()
            && let Err(e) = prepopulate_one(
                src.as_ref(),
                &save_path,
                parsed,
                &release.filename,
                opts.dry_run,
            )
        {
            warn!(file = %release.filename, error = %e, "prepopulate failed");
            stats.prepopulate_failed += 1;
        }
    }

    info!(
        queued = stats.queued,
        already_have = stats.already_have,
        already_queued = stats.already_queued,
        unparseable = stats.unparseable,
        no_crc = stats.no_crc,
        queue_failed = stats.queue_failed,
        prepopulate_failed = stats.prepopulate_failed,
        "done",
    );
    if stats.queue_failed > 0 {
        bail!(
            "{} torrent(s) failed to queue (see warnings above)",
            stats.queue_failed
        );
    }
    Ok(())
}

// ---------- helpers ----------

fn canonicalize_or_helpful_error(root: &Path) -> Result<PathBuf> {
    root.canonicalize().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            anyhow!("path does not exist: {}", root.display())
        } else {
            anyhow!("{}: {}", root.display(), e)
        }
    })
}

fn parse_resolution_cap(s: &str) -> Result<u32> {
    // Accept "1080p" / "1080" / "720p" etc.
    let trimmed = s.trim().trim_end_matches('p');
    trimmed
        .parse::<u32>()
        .with_context(|| format!("invalid --resolution {s:?}: expected `1080p`, `720p`, …"))
}

fn build_metadata_source(http: Rc<CachedHttp>) -> Rc<dyn DataSource> {
    Rc::new(Composite::new(vec![
        Rc::new(OnepaceNet::new(http.clone())),
        Rc::new(SpykerNz::new(http.clone())),
        Rc::new(GoogleSheet::new(http)),
    ]))
}

#[derive(Default)]
struct RunStats {
    queued: usize,
    already_have: usize,
    already_queued: usize,
    unparseable: usize,
    no_crc: usize,
    queue_failed: usize,
    prepopulate_failed: usize,
}

/// Group releases by (arc, episode), apply the resolution cap, and within
/// each group keep the highest-resolution release that still fits.
fn pick_best_per_episode(
    releases: Vec<Release>,
    max_height: u32,
    only_arc: Option<&str>,
) -> Vec<Release> {
    let only_arc_norm = only_arc.map(|s| s.to_ascii_lowercase());
    let mut buckets: HashMap<(String, u32), Vec<Release>> = HashMap::new();
    for r in releases {
        let Some(parsed) = r.parsed.as_ref() else {
            continue;
        };
        if let Some(needle) = &only_arc_norm
            && !parsed.arc.to_ascii_lowercase().contains(needle)
        {
            continue;
        }
        let Some(h) = r.height() else {
            continue;
        };
        if h > max_height {
            continue;
        }
        let key = (normalize_arc(&parsed.arc), parsed.episode);
        buckets.entry(key).or_default().push(r);
    }
    let mut out: Vec<Release> = buckets
        .into_values()
        .filter_map(|group| group.into_iter().max_by_key(|r| r.height().unwrap_or(0)))
        .collect();
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    out
}

fn library_crcs(root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "skipping unreadable path during library scan");
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.into_path();
        if !is_video(&path) {
            continue;
        }
        if let Some(parsed) = ParsedFile::from_path(&path)
            && let Some(crc) = parsed.crc32
        {
            out.insert(crc);
        }
    }
    out
}

/// Locate an existing arc folder under `root` that matches `arc_name`, or
/// propose a folder name to create. Uses normalize_arc for matching;
/// returns the *basename* of the destination folder relative to `root`.
fn find_or_propose_arc_folder(root: &Path, arc_name: &str) -> String {
    let target_norm = normalize_arc(arc_name);
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && let Some(name) = entry.file_name().to_str()
                && let Some(parsed) = crate::matcher::ParsedFile::from_filename(&format!(
                    "[One Pace][1] {name} 01 [1080p][00000000].mkv"
                ))
            {
                // Cheap reuse of ParsedFile to extract the arc from the
                // folder name pattern; if it matches we use that folder.
                if normalize_arc(&parsed.arc) == target_norm {
                    return name.to_string();
                }
            }
        }
        // Folder-name match fallback: just substring-compare normalized.
        for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
            if let Some(name) = entry.file_name().to_str()
                && normalize_arc(name).contains(&target_norm)
            {
                return name.to_string();
            }
        }
    }
    // Nothing matched; propose a fresh arc folder. Matches what users
    // typically see, e.g. "[One Pace][?] <Arc> [1080p]".
    format!("[One Pace] {arc_name}")
}

fn short_magnet(m: &str) -> String {
    if let Some(rest) = m.strip_prefix("magnet:?")
        && let Some(end) = rest.find('&')
    {
        return format!("magnet:?{}…", &rest[..end]);
    }
    m.chars().take(80).collect()
}

fn prepopulate_one(
    source: &dyn DataSource,
    save_path: &Path,
    parsed: &ParsedFile,
    filename: &str,
    dry_run: bool,
) -> Result<()> {
    let arc_norm = normalize_arc(&parsed.arc);
    let (arc_norm, episode_number) = parsed
        .crc32
        .as_deref()
        .and_then(|crc| source.identify_by_crc(crc).ok().flatten())
        .unwrap_or((arc_norm, parsed.episode));

    let Some(episode) = source
        .episode(&arc_norm, episode_number)
        .context("looking up episode metadata")?
    else {
        debug!(file = %filename, "no metadata for prepopulate — skipping");
        return Ok(());
    };

    // NFO target: `<save_path>/<basename>.nfo` (same convention as generate).
    let basename = Path::new(filename)
        .file_stem()
        .ok_or_else(|| anyhow!("filename {filename:?} has no stem"))?;
    let mut nfo_path = save_path.join(basename);
    nfo_path.set_extension("nfo");

    if nfo_path.exists() {
        debug!(path = %nfo_path.display(), "prepopulate skipped: NFO already exists");
        return Ok(());
    }

    if dry_run {
        info!(would_write = %nfo_path.display(), "[dry-run] prepopulate");
        return Ok(());
    }
    // Lock=true: pre-populated NFOs are authoritative; we don't want
    // Jellyfin overwriting them once the .mkv lands.
    writer::write_episode(&nfo_path, &episode, true)?;
    info!(path = %nfo_path.display(), "prepopulated episode.nfo");
    Ok(())
}

/// Best-effort sanity check: query qBittorrent's default save path and
/// see if our (host-side) library is anywhere near it. If they share no
/// common prefix and no `--save-path-map` was set, the user is almost
/// certainly in the "qbt in a container with a different mount table"
/// trap and our save_path will fail.
///
/// Heuristic and informational only — false positives are tolerable.
fn warn_if_save_path_likely_unreachable(
    client: &QbtClient,
    library_root: &Path,
    path_map: Option<&PathMap>,
) {
    if path_map.is_some() {
        return; // user opted in explicitly; trust them
    }
    let Ok(qbt_default) = client.default_save_path() else {
        return; // older qBittorrent or unreachable endpoint; skip the hint
    };
    if qbt_default.is_empty() {
        return;
    }
    let qbt_root = Path::new(&qbt_default);
    if library_root.starts_with(qbt_root) || qbt_root.starts_with(library_root) {
        return; // shared prefix — most likely native or same-mount setup
    }
    // Suggest a plausible mapping pair using the topmost components of
    // each path; the user can adjust.
    let host_hint =
        top_component(library_root).unwrap_or_else(|| library_root.display().to_string());
    let container_hint = top_component(qbt_root).unwrap_or_else(|| qbt_root.display().to_string());
    warn!(
        "qBittorrent's default save path is {qbt_default:?} but your library is at {} — \
         their paths don't share a prefix. If qBittorrent runs in a container or under \
         a different mount table, the save_path pacefinder sends will not be reachable. \
         Consider --save-path-map {}={} (adjust to match your setup).",
        library_root.display(),
        host_hint,
        container_hint,
    );
}

/// Return the leading path component (after the root `/`) as a string,
/// for use in the path-map hint message. `/mnt/media/x/y` → `/mnt`.
fn top_component(p: &Path) -> Option<String> {
    let mut comps = p.components();
    // Skip the root prefix if present.
    if let Some(std::path::Component::RootDir) = comps.next() {
        comps
            .next()
            .map(|c| format!("/{}", c.as_os_str().to_string_lossy()))
    } else {
        None
    }
}
