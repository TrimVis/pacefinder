//! `download` subcommand: queue missing One Pace releases for download.
//! See `docs/download.md` for the diff-and-queue flow.

use anyhow::{Context, Result, anyhow, bail};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::dl::Release;
use crate::dl::qbittorrent::QbtClient;
use crate::fs_util::canonicalize_root;
use crate::matcher::{ParsedFile, arc_from_folder_name, normalize_arc};
use crate::nfo::writer;
use crate::scan::is_video;
use crate::source::DataSource;
use crate::source::cache::CachedHttp;
use crate::source::default_chain;
use crate::source::onepacenet::OnepaceNet;

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
    pub requeue_existing: bool,
    pub prefer_extended: bool,
    pub only_arc: Option<String>,
    /// `HOST=CONTAINER` — translate save paths from pacefinder's view to
    /// qBittorrent's view when they differ (Docker mount tables etc.).
    pub save_path_map: Option<String>,
    pub fail_on_empty: bool,
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
    let root = canonicalize_root(root)?;
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
    if releases.is_empty() && opts.fail_on_empty {
        bail!("/releases returned zero magnets — likely upstream parse regression");
    }

    // 2 + 3. Filter by resolution + only_arc; collapse to best-per-episode.
    let chosen = pick_best_per_episode(
        releases,
        max_height,
        opts.only_arc.as_deref(),
        opts.prefer_extended,
    );
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
        (None, HashSet::new())
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
        let queued: HashSet<String> = names
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
        Some(default_chain(http.clone()))
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
        if !opts.requeue_existing && have.contains(crc) {
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
        // Translate to qBittorrent's view if a mapping was configured. The
        // startup `bail!` above guarantees the host prefix is a parent of
        // root, so translate cannot return None here under normal use.
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
            info!(
                magnet = %short_magnet(&release.magnet),
                save_path = %save_path.display(),
                host_path = %save_path_host.display(),
                file = %release.filename,
                "[dry-run] would queue",
            );
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
            // NFO writes happen on the host; use the host-side path even
            // when --save-path-map translated `save_path` for qBittorrent.
            && let Err(e) = prepopulate_one(
                src.as_ref(),
                &save_path_host,
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

fn parse_resolution_cap(s: &str) -> Result<u32> {
    // Accept "1080p" / "1080P" / "1080" / "720p" etc.
    let lower = s.trim().to_ascii_lowercase();
    let trimmed = lower.trim_end_matches('p');
    trimmed
        .parse::<u32>()
        .with_context(|| format!("invalid --resolution {s:?}: expected `1080p`, `720p`, …"))
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
///
/// When `prefer_extended` is false, Extended releases are filtered out
/// entirely — users only get them by opting in. When true, an Extended
/// release in a bucket wins over any regular sibling, regardless of
/// height (preference > resolution cap).
fn pick_best_per_episode(
    releases: Vec<Release>,
    max_height: u32,
    only_arc: Option<&str>,
    prefer_extended: bool,
) -> Vec<Release> {
    let only_arc_norm = only_arc.map(|s| s.to_ascii_lowercase());
    let mut buckets: HashMap<(String, u32), Vec<Release>> = HashMap::new();
    for r in releases {
        let Some(parsed) = r.parsed.as_ref() else {
            continue;
        };
        // Without opt-in, Extended cuts are out of scope.
        if !prefer_extended && parsed.extended {
            continue;
        }
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
        .filter_map(|group| {
            // With prefer_extended, any Extended in the bucket wins outright;
            // tiebreak among Extended-or-non-Extended siblings is max height.
            if prefer_extended
                && let Some(ext) = group
                    .iter()
                    .filter(|r| r.parsed.as_ref().is_some_and(|p| p.extended))
                    .max_by_key(|r| r.height().unwrap_or(0))
                    .cloned()
            {
                return Some(ext);
            }
            group.into_iter().max_by_key(|r| r.height().unwrap_or(0))
        })
        .collect();
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    out
}

fn library_crcs(root: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
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
/// propose a folder name to create. Returns the basename relative to `root`.
fn find_or_propose_arc_folder(root: &Path, arc_name: &str) -> String {
    let target_norm = normalize_arc(arc_name);
    let Ok(entries) = std::fs::read_dir(root) else {
        return format!("[One Pace] {arc_name}");
    };
    let dirs: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    // Exact: arc parsed out of a `[One Pace][...] <Arc> [<res>]` folder.
    for name in &dirs {
        if arc_from_folder_name(name)
            .map(|a| normalize_arc(&a) == target_norm)
            .unwrap_or(false)
        {
            return name.clone();
        }
    }
    // Fallback: substring match on normalized folder name (handles
    // unconventional folder shapes).
    for name in &dirs {
        if normalize_arc(name).contains(&target_norm) {
            return name.clone();
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // PathMap::parse uses a path that doesn't exist on disk so canonicalize
    // falls back to lexical — tests don't depend on the host filesystem.
    const NX: &str = "/nonexistent_pacefinder_test_dir/media";

    #[test]
    fn pathmap_parse_basic() {
        let m = PathMap::parse(&format!("{NX}=/downloads")).unwrap();
        assert_eq!(m.host, PathBuf::from(NX));
        assert_eq!(m.container, PathBuf::from("/downloads"));
    }

    #[test]
    fn pathmap_parse_strips_trailing_slashes() {
        let m = PathMap::parse(&format!("{NX}/=/downloads/")).unwrap();
        assert_eq!(m.host, PathBuf::from(NX));
        assert_eq!(m.container, PathBuf::from("/downloads"));
    }

    #[test]
    fn pathmap_parse_first_equals_splits() {
        // "a=b=c" is allowed: HOST="a", CONTAINER="b=c". Documents the
        // split_once contract.
        let m = PathMap::parse("/a=/b=/c").unwrap();
        assert_eq!(m.host, PathBuf::from("/a"));
        assert_eq!(m.container, PathBuf::from("/b=/c"));
    }

    #[test]
    fn pathmap_parse_rejects_missing_equals() {
        let err = PathMap::parse("no-equals").unwrap_err().to_string();
        assert!(err.contains("HOST=CONTAINER"), "got: {err}");
    }

    #[test]
    fn pathmap_parse_rejects_empty_sides() {
        assert!(PathMap::parse("=/c").is_err());
        assert!(PathMap::parse("/h=").is_err());
        assert!(PathMap::parse("=").is_err());
        // "/=/" trims to empty on both sides — rejected.
        assert!(PathMap::parse("/=/").is_err());
    }

    #[test]
    fn pathmap_translate_strips_host_prefix() {
        let m = PathMap {
            host: PathBuf::from("/mnt/media"),
            container: PathBuf::from("/downloads"),
        };
        assert_eq!(
            m.translate(Path::new("/mnt/media/Arc/file.mkv")),
            Some(PathBuf::from("/downloads/Arc/file.mkv")),
        );
    }

    #[test]
    fn pathmap_translate_exact_match() {
        let m = PathMap {
            host: PathBuf::from("/mnt/media"),
            container: PathBuf::from("/downloads"),
        };
        assert_eq!(
            m.translate(Path::new("/mnt/media")),
            Some(PathBuf::from("/downloads")),
        );
    }

    #[test]
    fn pathmap_translate_component_aware() {
        // /mnt/medianext doesn't start with /mnt/media in a path-component
        // sense — strip_prefix correctly returns None.
        let m = PathMap {
            host: PathBuf::from("/mnt/media"),
            container: PathBuf::from("/downloads"),
        };
        assert!(m.translate(Path::new("/mnt/medianext/x")).is_none());
    }

    #[test]
    fn pathmap_translate_outside_host_is_none() {
        let m = PathMap {
            host: PathBuf::from("/mnt/media"),
            container: PathBuf::from("/downloads"),
        };
        assert!(m.translate(Path::new("/other/path")).is_none());
    }

    #[test]
    fn parse_resolution_cap_handles_common_forms() {
        assert_eq!(parse_resolution_cap("1080p").unwrap(), 1080);
        assert_eq!(parse_resolution_cap("720p").unwrap(), 720);
        assert_eq!(parse_resolution_cap("1080").unwrap(), 1080);
        assert_eq!(parse_resolution_cap("  480p  ").unwrap(), 480);
    }

    #[test]
    fn parse_resolution_cap_accepts_uppercase_p() {
        assert_eq!(parse_resolution_cap("1080P").unwrap(), 1080);
    }

    #[test]
    fn parse_resolution_cap_rejects_garbage() {
        assert!(parse_resolution_cap("").is_err());
        assert!(parse_resolution_cap("garbage").is_err());
        assert!(parse_resolution_cap("4k").is_err());
        assert!(parse_resolution_cap("1080p60").is_err());
    }

    #[test]
    fn short_magnet_truncates_at_ampersand() {
        let m = "magnet:?xt=urn:btih:abc&dn=foo&tr=bar";
        assert_eq!(short_magnet(m), "magnet:?xt=urn:btih:abc…");
    }

    #[test]
    fn short_magnet_passes_through_short_input() {
        let m = "magnet:?xt=urn:btih:abc";
        assert_eq!(short_magnet(m), m);
    }

    #[test]
    fn short_magnet_caps_long_non_magnet_at_80_chars() {
        let out = short_magnet(&"x".repeat(120));
        assert_eq!(out.len(), 80);
    }

    #[test]
    fn top_component_returns_first_under_root() {
        assert_eq!(
            top_component(Path::new("/mnt/media/anime")),
            Some("/mnt".into()),
        );
        assert_eq!(top_component(Path::new("/foo")), Some("/foo".into()));
    }

    #[test]
    fn top_component_none_for_root_only() {
        assert!(top_component(Path::new("/")).is_none());
    }

    #[test]
    fn top_component_none_for_relative_path() {
        assert!(top_component(Path::new("relative/path")).is_none());
    }

    fn release(arc: &str, ep: u32, res: &str, crc: &str) -> Release {
        Release {
            magnet: format!("magnet:?xt=urn:btih:{crc}"),
            filename: format!("[One Pace][1] {arc} {ep:02} [{res}][{crc}].mkv"),
            parsed: Some(ParsedFile {
                arc: arc.into(),
                episode: ep,
                crc32: Some(crc.into()),
                resolution: Some(res.into()),
                extended: false,
            }),
        }
    }

    #[test]
    fn pick_best_picks_highest_within_cap() {
        let chosen = pick_best_per_episode(
            vec![
                release("Wano", 1, "480p", "00000001"),
                release("Wano", 1, "720p", "00000002"),
                release("Wano", 1, "1080p", "00000003"),
            ],
            1080,
            None,
            false,
        );
        assert_eq!(chosen.len(), 1);
        assert_eq!(
            chosen[0].parsed.as_ref().unwrap().resolution.as_deref(),
            Some("1080p"),
        );
    }

    #[test]
    fn pick_best_drops_above_cap() {
        let chosen = pick_best_per_episode(
            vec![
                release("Wano", 1, "720p", "AAA"),
                release("Wano", 1, "1080p", "BBB"),
            ],
            720,
            None,
            false,
        );
        assert_eq!(chosen.len(), 1);
        assert_eq!(
            chosen[0].parsed.as_ref().unwrap().resolution.as_deref(),
            Some("720p"),
        );
    }

    #[test]
    fn pick_best_empty_when_all_above_cap() {
        let chosen =
            pick_best_per_episode(vec![release("Wano", 1, "1080p", "AAA")], 480, None, false);
        assert!(chosen.is_empty());
    }

    #[test]
    fn pick_best_keeps_distinct_episodes() {
        let chosen = pick_best_per_episode(
            vec![
                release("Wano", 1, "1080p", "AAA"),
                release("Wano", 2, "1080p", "BBB"),
            ],
            1080,
            None,
            false,
        );
        assert_eq!(chosen.len(), 2);
    }

    #[test]
    fn pick_best_only_arc_substring_case_insensitive() {
        let chosen = pick_best_per_episode(
            vec![
                release("Wano Act 1", 1, "1080p", "AAA"),
                release("Romance Dawn", 1, "1080p", "BBB"),
            ],
            1080,
            Some("wano"),
            false,
        );
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].parsed.as_ref().unwrap().arc, "Wano Act 1");
    }

    #[test]
    fn pick_best_skips_parseless_releases() {
        let chosen = pick_best_per_episode(
            vec![
                Release {
                    magnet: "magnet:?xt=urn:btih:xxx".into(),
                    filename: "garbage.mkv".into(),
                    parsed: None,
                },
                release("Wano", 1, "1080p", "AAA"),
            ],
            1080,
            None,
            false,
        );
        assert_eq!(chosen.len(), 1);
    }

    #[test]
    fn pick_best_groups_by_normalized_arc() {
        // "Whiskey Peak" and "whiskey-peak" should bucket together via
        // normalize_arc; one wins.
        let chosen = pick_best_per_episode(
            vec![
                release("Whiskey Peak", 1, "720p", "AAA"),
                release("whiskey-peak", 1, "1080p", "BBB"),
            ],
            1080,
            None,
            false,
        );
        assert_eq!(chosen.len(), 1);
    }

    fn extended(arc: &str, ep: u32, res: &str, crc: &str) -> Release {
        let mut r = release(arc, ep, res, crc);
        if let Some(p) = r.parsed.as_mut() {
            p.extended = true;
        }
        r
    }

    #[test]
    fn pick_best_excludes_extended_without_opt_in() {
        // Default behavior: Extended releases are filtered out entirely.
        let chosen = pick_best_per_episode(
            vec![
                extended("Wano", 5, "1080p", "EXT00001"),
                release("Wano", 5, "720p", "REG00001"),
            ],
            1080,
            None,
            false,
        );
        assert_eq!(chosen.len(), 1);
        assert_eq!(
            chosen[0].parsed.as_ref().unwrap().crc32.as_deref(),
            Some("REG00001")
        );
    }

    #[test]
    fn pick_best_prefers_extended_over_higher_res_regular() {
        // Preference > resolution: 720p Extended beats 1080p regular.
        let chosen = pick_best_per_episode(
            vec![
                release("Wano", 5, "1080p", "REG00001"),
                extended("Wano", 5, "720p", "EXT00001"),
            ],
            1080,
            None,
            true,
        );
        assert_eq!(chosen.len(), 1);
        let p = chosen[0].parsed.as_ref().unwrap();
        assert!(p.extended);
        assert_eq!(p.crc32.as_deref(), Some("EXT00001"));
    }

    #[test]
    fn pick_best_uses_regular_when_no_extended_with_opt_in() {
        // Opt-in alone doesn't conjure Extended where none exists.
        let chosen = pick_best_per_episode(
            vec![release("Wano", 5, "1080p", "REG00001")],
            1080,
            None,
            true,
        );
        assert_eq!(chosen.len(), 1);
        assert!(!chosen[0].parsed.as_ref().unwrap().extended);
    }

    #[test]
    fn pick_best_extended_still_subject_to_cap() {
        // Extended above cap is still dropped.
        let chosen = pick_best_per_episode(
            vec![extended("Wano", 5, "1080p", "EXT00001")],
            720,
            None,
            true,
        );
        assert!(chosen.is_empty());
    }

    #[test]
    fn find_or_propose_arc_folder_exact_match() {
        let dir = tempdir().unwrap();
        let arc = "[One Pace][1-7] Romance Dawn [1080p]";
        std::fs::create_dir_all(dir.path().join(arc)).unwrap();
        assert_eq!(find_or_propose_arc_folder(dir.path(), "Romance Dawn"), arc,);
    }

    #[test]
    fn find_or_propose_arc_folder_substring_fallback() {
        let dir = tempdir().unwrap();
        // Non-canonical name — only substring match catches it.
        std::fs::create_dir_all(dir.path().join("romance dawn")).unwrap();
        assert_eq!(
            find_or_propose_arc_folder(dir.path(), "Romance Dawn"),
            "romance dawn",
        );
    }

    #[test]
    fn find_or_propose_arc_folder_proposes_when_missing() {
        let dir = tempdir().unwrap();
        assert_eq!(
            find_or_propose_arc_folder(dir.path(), "Egghead"),
            "[One Pace] Egghead",
        );
    }
}
