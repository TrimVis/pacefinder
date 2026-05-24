//! Pluggable metadata data sources.
//!
//! A `DataSource` knows how to answer queries about One Pace's series,
//! seasons, episodes, and images. Three adapters ship in-tree
//! (`onepacenet`, `spykernz`, `sheet`); `default_chain` composes them
//! via `composite::Composite` so callers see one combined source.

use anyhow::Result;
use std::rc::Rc;

use crate::matcher::{ParsedFile, normalize_arc};
use crate::model::{Episode, Season, Series};

pub mod cache;
pub mod composite;
pub mod onepacenet;
pub mod sheet;
pub mod spykernz;

/// Canonical (arc_normalized, episode) for a parsed file, preferring the
/// source chain's CRC oracle (the Google Sheet today) when the filename
/// carries a CRC and the oracle answers. Falls back to the filename's own
/// arc + episode, normalized.
pub fn identify_or_fallback(source: &dyn DataSource, parsed: &ParsedFile) -> (String, u32) {
    parsed
        .crc32
        .as_deref()
        .and_then(|crc| source.identify_by_crc(crc).ok().flatten())
        .unwrap_or_else(|| (normalize_arc(&parsed.arc), parsed.episode))
}

/// Default composite of upstreams. Order:
/// - onepace.net first — current arc list + fresh season descriptions
/// - SpykerNZ second — rich episode titles/plots + series + posters
/// - GoogleSheet third — CRC-keyed file identification + synthesized
///   episode fallback for arcs SpykerNZ doesn't cover
pub fn default_chain(http: Rc<cache::CachedHttp>) -> Rc<dyn DataSource> {
    Rc::new(composite::Composite::new(vec![
        Rc::new(onepacenet::OnepaceNet::new(http.clone())),
        Rc::new(spykernz::SpykerNz::new(http.clone())),
        Rc::new(sheet::GoogleSheet::new(http)),
    ]))
}

/// Which on-disk image kind the caller wants. Lives next to the trait that
/// dispatches on it rather than in `model` — it's an adapter selector, not
/// a piece of domain data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageKind {
    SeriesPoster,
    SeasonPoster { number: u32 },
}

pub trait DataSource {
    /// Human-readable adapter name, useful in logs.
    fn name(&self) -> &'static str;

    /// Series-level metadata (titles, plot, named-season map). Returns
    /// `Ok(None)` if this source has no series-level info.
    fn series(&self) -> Result<Option<Series>>;

    /// Per-season metadata. Returns `Ok(None)` if the source has no record
    /// of the requested season number.
    fn season(&self, number: u32) -> Result<Option<Season>>;

    /// Episode metadata keyed by normalized arc name + 1-based episode number.
    /// Returns `Ok(None)` if the source has no record of that episode.
    fn episode(&self, arc_normalized: &str, episode_number: u32) -> Result<Option<Episode>>;

    /// Image bytes for the given kind, if available.
    fn image(&self, kind: ImageKind) -> Result<Option<Vec<u8>>>;

    /// Map a file's CRC32 (uppercase hex) to its canonical
    /// `(normalized_arc_name, episode_number)` if this source has such an
    /// index. Default returns `Ok(None)` — only the Google Sheet adapter
    /// has CRC-keyed data today.
    fn identify_by_crc(&self, _crc: &str) -> Result<Option<(String, u32)>> {
        Ok(None)
    }
}
