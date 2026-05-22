//! Composite data source: tries each underlying adapter in order.
//!
//! For every method, the first underlying source that returns `Some(...)`
//! wins. This lets you put a fresh-but-incomplete source first (e.g.
//! onepace.net for season descriptions) and a comprehensive-but-stale
//! source second (e.g. SpykerNZ for episode-level data and posters).

use std::sync::Arc;

use anyhow::Result;
use tracing::warn;

use super::{DataSource, ImageKind};
use crate::model::{Episode, Season, Series};

pub struct Composite {
    sources: Vec<Arc<dyn DataSource>>,
}

impl Composite {
    pub fn new(sources: Vec<Arc<dyn DataSource>>) -> Self {
        Self { sources }
    }

    /// Try each source in order; return the first `Some`, log and skip on
    /// `Err`, fall through on `Ok(None)`. `op` is included in the warning
    /// so a failure pinpoints which method drifted.
    fn try_each<T>(
        &self,
        op: &'static str,
        mut f: impl FnMut(&dyn DataSource) -> Result<Option<T>>,
    ) -> Result<Option<T>> {
        for s in &self.sources {
            match f(s.as_ref()) {
                Ok(Some(v)) => return Ok(Some(v)),
                Ok(None) => continue,
                Err(e) => warn!(source = s.name(), op, error = %e, "fetch failed"),
            }
        }
        Ok(None)
    }
}

impl DataSource for Composite {
    fn name(&self) -> &'static str {
        "composite"
    }

    fn series(&self) -> Result<Option<Series>> {
        self.try_each("series", |s| s.series())
    }

    fn season(&self, number: u32) -> Result<Option<Season>> {
        self.try_each("season", |s| s.season(number))
    }

    fn episode(&self, arc_normalized: &str, episode_number: u32) -> Result<Option<Episode>> {
        self.try_each("episode", |s| s.episode(arc_normalized, episode_number))
    }

    fn image(&self, kind: ImageKind) -> Result<Option<Vec<u8>>> {
        self.try_each("image", |s| s.image(kind))
    }
}
