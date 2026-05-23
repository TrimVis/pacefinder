//! Adapter for a community-tracked One Pace Google Sheet.
//!
//! The spreadsheet tracks per-arc episode lists keyed
//! by MKV CRC32. We use it for two purposes:
//!
//! 1. As a `DataSource`, synthesizing minimal `Episode` records from the
//!    chapter / anime-episode / release-date columns when no richer source
//!    (SpykerNZ, onepace.net) has data for that arc.
//! 2. As a CRC-to-(arc, episode) oracle via [`GoogleSheet::lookup_arc_ep_by_crc`].
//!    The `generate` command uses this to override filename-derived arc and
//!    episode numbers with whatever the sheet says — useful when a release
//!    has been re-cut and the CRC moves to a different slot.
//!
//! The sheet has one summary tab (gid=0, no CRC column, ignored) plus one
//! tab per arc. We discover tab gids by scraping the `/preview` page (which
//! is User-Agent-gated so we identify as a browser), then fetch each tab's
//! data via the public `gviz/tq` JSONP endpoint.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow};
use regex_lite::Regex;
use serde::Deserialize;
use tracing::{debug, warn};

use super::cache::CachedHttp;
use super::{DataSource, ImageKind};
use crate::matcher::normalize_arc;
use crate::model::{Episode, Season, Series};

const SPREADSHEET_ID: &str = "1HQRMJgu_zArp-sLnvFMDzOyjdsht87eFLECxMK858lA";
const PREVIEW_UA: &str = "Mozilla/5.0";

pub struct GoogleSheet {
    http: Rc<CachedHttp>,
    index: OnceCell<Rc<SheetIndex>>,
}

impl GoogleSheet {
    pub fn new(http: Rc<CachedHttp>) -> Self {
        Self {
            http,
            index: OnceCell::new(),
        }
    }

    /// Returns `(normalized_arc_name, episode_number_within_arc)` if `crc`
    /// (uppercase hex) appears in any per-arc tab's `MKV CRC32` or
    /// `MKV CRC32 (Extended)` column.
    pub fn lookup_arc_ep_by_crc(&self, crc: &str) -> Result<Option<(String, u32)>> {
        let index = self.ensure_index()?;
        let key = crc.to_ascii_uppercase();
        Ok(index
            .by_crc
            .get(&key)
            .map(|ep| (ep.arc_normalized.clone(), ep.episode_number)))
    }

    fn ensure_index(&self) -> Result<Rc<SheetIndex>> {
        if let Some(idx) = self.index.get() {
            return Ok(Rc::clone(idx));
        }
        let idx = Rc::new(self.build_index()?);
        let _ = self.index.set(Rc::clone(&idx));
        Ok(Rc::clone(self.index.get().expect("just set")))
    }

    fn build_index(&self) -> Result<SheetIndex> {
        let preview_url =
            format!("https://docs.google.com/spreadsheets/d/{SPREADSHEET_ID}/preview");
        let html = self
            .http
            .get_string_with_header(&preview_url, "User-Agent", PREVIEW_UA)
            .context("fetching google sheet preview page")?;
        let gids = extract_gids(&html);
        debug!(count = gids.len(), "discovered sheet gids");

        let mut index = SheetIndex::default();
        for gid in gids {
            if gid == 0 {
                // Summary tab — no CRC column.
                continue;
            }
            let url = format!(
                "https://docs.google.com/spreadsheets/d/{SPREADSHEET_ID}/gviz/tq?tqx=out:json&gid={gid}"
            );
            let body = match self.http.get_string(&url) {
                Ok(b) => b,
                Err(e) => {
                    warn!(gid, error = %e, "failed to fetch sheet tab");
                    continue;
                }
            };
            match parse_arc_tab(&body) {
                Ok(entries) => index.absorb(entries),
                Err(e) => {
                    warn!(gid, error = %e, "failed to parse sheet tab");
                }
            }
        }
        index.finalize();
        Ok(index)
    }
}

impl DataSource for GoogleSheet {
    fn name(&self) -> &'static str {
        "google-sheet"
    }

    fn series(&self) -> Result<Option<Series>> {
        Ok(None)
    }

    fn season(&self, _number: u32) -> Result<Option<Season>> {
        Ok(None)
    }

    fn image(&self, _kind: ImageKind) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    fn episode(&self, arc_normalized: &str, episode_number: u32) -> Result<Option<Episode>> {
        let index = self.ensure_index()?;
        // Try the name as-is, then a small alias map (handles
        // Arabasta/Alabasta and Wano Act 1/Wano spelling drift between
        // the user's library and the sheet).
        let canonical = arc_alias(arc_normalized).unwrap_or(arc_normalized);
        let Some(arc_entries) = index.by_arc.get(canonical) else {
            return Ok(None);
        };
        let Some(entry) = arc_entries
            .iter()
            .find(|e| e.episode_number == episode_number)
        else {
            return Ok(None);
        };
        Ok(Some(synthesize_episode(entry)))
    }

    fn identify_by_crc(&self, crc: &str) -> Result<Option<(String, u32)>> {
        self.lookup_arc_ep_by_crc(crc)
    }
}

/// Spelling drift between user folder names and the sheet's canonical
/// arc names. Apply on lookup; keep small.
fn arc_alias(normalized: &str) -> Option<&'static str> {
    match normalized {
        "arabasta" => Some("alabasta"),
        "wano act 1" => Some("wano"),
        _ => None,
    }
}

// ---------- index ----------

#[derive(Debug, Default)]
struct SheetIndex {
    by_crc: HashMap<String, SheetEpisode>,
    by_arc: HashMap<String, Vec<SheetEpisode>>,
}

impl SheetIndex {
    fn absorb(&mut self, entries: Vec<ParsedRow>) {
        for row in entries {
            let entry = SheetEpisode {
                arc: row.arc.clone(),
                arc_normalized: normalize_arc(&row.arc),
                episode_label: row.episode_label,
                episode_number: row.episode_number,
                chapters: row.chapters,
                anime_episodes: row.anime_episodes,
                release_date: row.release_date,
            };
            for crc in row.crcs {
                let key = crc.to_ascii_uppercase();
                self.by_crc.insert(key, entry.clone());
            }
            self.by_arc
                .entry(entry.arc_normalized.clone())
                .or_default()
                .push(entry);
        }
    }

    fn finalize(&mut self) {
        for v in self.by_arc.values_mut() {
            v.sort_by_key(|e| e.episode_number);
        }
    }
}

#[derive(Debug, Clone)]
struct SheetEpisode {
    /// Original display form (e.g. "Skypiea", "Egghead"). Used to build
    /// synthesized episode titles with proper capitalization.
    arc: String,
    arc_normalized: String,
    #[allow(dead_code)]
    episode_label: String,
    episode_number: u32,
    chapters: String,
    anime_episodes: String,
    release_date: Option<String>,
}

/// Intermediate per-row result of parsing a single arc tab.
#[derive(Debug)]
struct ParsedRow {
    arc: String,
    episode_label: String,
    episode_number: u32,
    chapters: String,
    anime_episodes: String,
    release_date: Option<String>,
    crcs: Vec<String>,
}

// ---------- parsing ----------

static GID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"gid=(\d+)").unwrap());
static JSONP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^[^(]*\((.*)\);?\s*$").unwrap());
// "Skypiea 03", "Long Ring Long Land 02", "Wano 12" — trailing 1-3 digits.
static TRAILING_NUM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{1,3})\s*$").unwrap());
// "Wano 01", "Wano XX" — leading label form used as a fallback arc source.
static LEADING_LABEL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z][A-Za-z' \-]*?)\s+\d{1,3}\s*$").unwrap());

/// Extract unique gids from the preview HTML.
fn extract_gids(html: &str) -> Vec<u32> {
    let mut gids: Vec<u32> = GID_RE
        .captures_iter(html)
        .filter_map(|c| c.get(1)?.as_str().parse().ok())
        .collect();
    gids.sort_unstable();
    gids.dedup();
    gids
}

#[derive(Debug, Deserialize)]
struct GvizTable {
    cols: Vec<GvizCol>,
    rows: Vec<GvizRow>,
}

#[derive(Debug, Deserialize)]
struct GvizCol {
    #[serde(default)]
    label: String,
}

#[derive(Debug, Deserialize)]
struct GvizRow {
    #[serde(default)]
    c: Vec<Option<GvizCell>>,
}

#[derive(Debug, Deserialize)]
struct GvizCell {
    #[serde(default, rename = "v")]
    v: serde_json::Value,
    #[serde(default, rename = "f")]
    f: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GvizEnvelope {
    table: GvizTable,
}

/// Parse one arc tab's JSONP body. Returns the per-row data extracted from
/// the tab, or `Err` if the tab is unusable (no CRC column, malformed
/// payload). Skips rows that lack a usable episode label or CRC.
fn parse_arc_tab(body: &str) -> Result<Vec<ParsedRow>> {
    let caps = JSONP_RE
        .captures(body.trim())
        .ok_or_else(|| anyhow!("not a JSONP envelope"))?;
    let json = caps.get(1).unwrap().as_str();
    let env: GvizEnvelope = serde_json::from_str(json).context("parsing gviz JSON")?;

    let header_arc = env.table.cols.first().map(|c| c.label.trim().to_string());
    let ep_col = find_col(&env.table.cols, "One Pace Episode")
        .ok_or_else(|| anyhow!("no 'One Pace Episode' column"))?;
    let chapters_col = find_col(&env.table.cols, "Chapters");
    let episodes_col = find_col(&env.table.cols, "Episodes");
    let release_col = find_col(&env.table.cols, "Release Date");
    let crc_col =
        find_col(&env.table.cols, "MKV CRC32").ok_or_else(|| anyhow!("no MKV CRC32 column"))?;
    let crc_ext_col = find_col(&env.table.cols, "MKV CRC32 (Extended)");

    let mut out = Vec::with_capacity(env.table.rows.len());
    for row in &env.table.rows {
        let Some(label) = cell_str(row, ep_col) else {
            continue;
        };
        let label = label.trim().to_string();
        if label.is_empty() {
            continue;
        }
        let Some(episode_number) = parse_episode_number(&label) else {
            continue;
        };

        let arc = pick_arc(header_arc.as_deref(), &label).unwrap_or_else(|| label.clone());

        let chapters = cell_str(row, chapters_col.unwrap_or(usize::MAX)).unwrap_or_default();
        let anime_episodes = cell_str(row, episodes_col.unwrap_or(usize::MAX)).unwrap_or_default();
        let release_date = release_col
            .and_then(|i| cell_str(row, i))
            .and_then(|s| normalize_release_date(&s));

        let mut crcs = Vec::new();
        if let Some(crc) = cell_str(row, crc_col)
            && !crc.trim().is_empty()
        {
            crcs.push(crc.trim().to_string());
        }
        if let Some(i) = crc_ext_col
            && let Some(crc) = cell_str(row, i)
            && !crc.trim().is_empty()
        {
            crcs.push(crc.trim().to_string());
        }

        out.push(ParsedRow {
            arc,
            episode_label: label,
            episode_number,
            chapters,
            anime_episodes,
            release_date,
            crcs,
        });
    }
    Ok(out)
}

fn find_col(cols: &[GvizCol], label: &str) -> Option<usize> {
    cols.iter().position(|c| c.label.trim() == label)
}

/// Stringify a cell, preferring the formatted display value (`f`) so we get
/// "Ep. 153" rather than the raw stringified number.
fn cell_str(row: &GvizRow, idx: usize) -> Option<String> {
    let cell = row.c.get(idx)?.as_ref()?;
    if let Some(f) = &cell.f
        && !f.is_empty()
    {
        return Some(f.clone());
    }
    match &cell.v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn parse_episode_number(label: &str) -> Option<u32> {
    TRAILING_NUM_RE
        .captures(label)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

/// Prefer the column header for the arc display name; if absent (the
/// Wano-style tab with an empty first-column header), recover it from the
/// leading text of the episode label.
fn pick_arc(header: Option<&str>, label: &str) -> Option<String> {
    if let Some(h) = header
        && !h.is_empty()
    {
        return Some(h.to_string());
    }
    LEADING_LABEL_RE
        .captures(label)
        .map(|c| c.get(1).unwrap().as_str().trim().to_string())
}

/// `"2025.05.03"` → `"2025-05-03"`. Returns `None` if the input is not a
/// well-formed three-part dotted date.
fn normalize_release_date(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let y: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1900 {
        return None;
    }
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

fn synthesize_episode(entry: &SheetEpisode) -> Episode {
    let title = format!("{} {:02}", entry.arc, entry.episode_number);
    let release_suffix = entry
        .release_date
        .as_deref()
        .map(|d| format!(" Released {d}."))
        .unwrap_or_default();
    let plot = Some(format!(
        "Manga: {}. Original anime: {}.{}",
        entry.chapters, entry.anime_episodes, release_suffix
    ));
    Episode {
        showtitle: "One Pace".to_string(),
        season: 0,
        number: entry.episode_number,
        title,
        plot,
        premiered: entry.release_date.clone(),
        aired: entry.release_date.clone(),
    }
}

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_jsonp() -> String {
        // Mimic the gviz JSONP envelope. The first column header carries
        // the arc name ("Skypiea"). Two rows: one with extended CRC, one
        // without; plus a row missing a CRC entirely (should be parsed for
        // episode listing but contribute no CRC entries).
        let json = r#"{
            "table": {
                "cols": [
                    {"label": "Skypiea"},
                    {"label": "One Pace Episode"},
                    {"label": "Chapters"},
                    {"label": "Episodes"},
                    {"label": "Release Date"},
                    {"label": "Length"},
                    {"label": "MKV CRC32"},
                    {"label": "MKV CRC32 (Extended)"}
                ],
                "rows": [
                    {"c": [
                        null,
                        {"v": "Skypiea 01"},
                        {"v": "Ch. 237-238"},
                        {"v": "Ep. 153"},
                        {"v": "2023.06.12"},
                        {"v": "x"},
                        {"v": "ca5552fc"},
                        {"v": "DEADBEEF"}
                    ]},
                    {"c": [
                        null,
                        {"v": "Skypiea 02"},
                        {"v": "Ch. 239-240"},
                        {"v": "Ep. 154"},
                        {"v": "2023.07.01"},
                        {"v": "x"},
                        {"v": "ABCD1234"},
                        null
                    ]}
                ]
            }
        }"#;
        format!("/*O_o*/google.visualization.Query.setResponse({json});")
    }

    fn make_index() -> SheetIndex {
        let body = sample_jsonp();
        let rows = parse_arc_tab(&body).unwrap();
        let mut idx = SheetIndex::default();
        idx.absorb(rows);
        idx.finalize();
        idx
    }

    #[test]
    fn parse_arc_tab_extracts_rows_and_crcs() {
        let rows = parse_arc_tab(&sample_jsonp()).unwrap();
        assert_eq!(rows.len(), 2);

        let r0 = &rows[0];
        assert_eq!(r0.arc, "Skypiea");
        assert_eq!(r0.episode_label, "Skypiea 01");
        assert_eq!(r0.episode_number, 1);
        assert_eq!(r0.chapters, "Ch. 237-238");
        assert_eq!(r0.anime_episodes, "Ep. 153");
        assert_eq!(r0.release_date.as_deref(), Some("2023-06-12"));
        assert_eq!(r0.crcs.len(), 2);
        // CRCs are stored as-given here; uppercasing happens at index time.
        assert!(r0.crcs.iter().any(|c| c.eq_ignore_ascii_case("ca5552fc")));
        assert!(r0.crcs.iter().any(|c| c.eq_ignore_ascii_case("deadbeef")));

        let r1 = &rows[1];
        assert_eq!(r1.episode_number, 2);
        assert_eq!(r1.crcs, vec!["ABCD1234".to_string()]);
    }

    #[test]
    fn lookup_arc_ep_by_crc_finds_episodes() {
        let idx = make_index();

        // Normal CRC, uppercased on lookup.
        let hit = idx.by_crc.get("CA5552FC").unwrap();
        assert_eq!(hit.arc_normalized, "skypiea");
        assert_eq!(hit.episode_number, 1);

        // Extended CRC for the same row.
        let ext = idx.by_crc.get("DEADBEEF").unwrap();
        assert_eq!(ext.arc_normalized, "skypiea");
        assert_eq!(ext.episode_number, 1);

        // Second-row CRC.
        let r2 = idx.by_crc.get("ABCD1234").unwrap();
        assert_eq!(r2.episode_number, 2);

        // Unknown CRC.
        assert!(!idx.by_crc.contains_key("FFFFFFFF"));
    }

    #[test]
    fn episode_synthesizes_for_known_arc() {
        let idx = make_index();
        let entry = idx
            .by_arc
            .get("skypiea")
            .and_then(|v| v.iter().find(|e| e.episode_number == 1))
            .unwrap();
        let ep = synthesize_episode(entry);
        assert_eq!(ep.title, "Skypiea 01");
        assert_eq!(ep.showtitle, "One Pace");
        assert_eq!(ep.season, 0);
        assert_eq!(ep.number, 1);
        let plot = ep.plot.unwrap();
        assert!(plot.contains("Manga:"));
        assert!(plot.contains("Original anime:"));
        assert!(plot.contains("Released 2023-06-12."));
        assert_eq!(ep.premiered.as_deref(), Some("2023-06-12"));
        assert_eq!(ep.aired.as_deref(), Some("2023-06-12"));
    }

    #[test]
    fn episode_returns_none_for_unknown_arc() {
        let idx = make_index();
        assert!(!idx.by_arc.contains_key("nope"));
    }

    #[test]
    fn normalize_arc_used_consistently() {
        let idx = make_index();
        // Index key matches what `normalize_arc` produces for the display arc.
        assert!(idx.by_arc.contains_key(&normalize_arc("Skypiea")));
        // CRC entry's `arc_normalized` is the same normalized form.
        let hit = idx.by_crc.get("CA5552FC").unwrap();
        assert_eq!(hit.arc_normalized, normalize_arc("Skypiea"));
    }

    #[test]
    fn date_normalizes_dotted_to_iso() {
        assert_eq!(
            normalize_release_date("2025.05.03").as_deref(),
            Some("2025-05-03")
        );
        assert_eq!(
            normalize_release_date("2023.6.12").as_deref(),
            Some("2023-06-12")
        );
        assert!(normalize_release_date("2025/05/03").is_none());
        assert!(normalize_release_date("not a date").is_none());
        assert!(normalize_release_date("").is_none());
        assert!(normalize_release_date("2025.13.01").is_none());
        assert!(normalize_release_date("2025.05.32").is_none());
    }

    #[test]
    fn parse_episode_number_handles_multi_word_arcs() {
        assert_eq!(parse_episode_number("Skypiea 03"), Some(3));
        assert_eq!(parse_episode_number("Long Ring Long Land 02"), Some(2));
        assert_eq!(parse_episode_number("Wano 12"), Some(12));
        assert_eq!(parse_episode_number("No digits"), None);
    }

    #[test]
    fn pick_arc_falls_back_to_label_when_header_empty() {
        assert_eq!(pick_arc(Some(""), "Wano 01").as_deref(), Some("Wano"));
        assert_eq!(
            pick_arc(None, "Long Ring Long Land 04").as_deref(),
            Some("Long Ring Long Land")
        );
        assert_eq!(
            pick_arc(Some("Egghead"), "Egghead 02").as_deref(),
            Some("Egghead")
        );
    }

    #[test]
    fn extract_gids_dedupes_and_sorts() {
        let html = "stuff gid=0 more gid=12345 again gid=0 then gid=999";
        let gids = extract_gids(html);
        assert_eq!(gids, vec![0, 999, 12345]);
    }

    #[test]
    fn parse_arc_tab_rejects_payload_without_crc_column() {
        let json = r#"{
            "table": {
                "cols": [
                    {"label": "Summary"},
                    {"label": "One Pace Episode"}
                ],
                "rows": []
            }
        }"#;
        let body = format!("/*O_o*/google.visualization.Query.setResponse({json});");
        assert!(parse_arc_tab(&body).is_err());
    }
}
