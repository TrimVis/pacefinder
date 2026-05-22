//! Composite data source: tries each underlying adapter in order.
//!
//! For every method, the first underlying source that returns `Some(...)`
//! wins. This lets you put a fresh-but-incomplete source first (e.g.
//! onepace.net for season descriptions) and a comprehensive-but-stale
//! source second (e.g. SpykerNZ for episode-level data and posters).

use std::sync::Arc;

use anyhow::Result;
use tracing::warn;

use super::DataSource;
use crate::model::{Episode, ImageKind, Season, Series};

pub struct Composite {
    sources: Vec<Arc<dyn DataSource>>,
}

impl Composite {
    pub fn new(sources: Vec<Arc<dyn DataSource>>) -> Self {
        Self { sources }
    }
}

impl DataSource for Composite {
    fn name(&self) -> &'static str {
        "composite"
    }

    fn series(&self) -> Result<Option<Series>> {
        for s in &self.sources {
            match s.series() {
                Ok(Some(v)) => return Ok(Some(v)),
                Ok(None) => continue,
                Err(e) => warn!(source = s.name(), error = %e, "series fetch failed"),
            }
        }
        Ok(None)
    }

    fn season(&self, number: u32) -> Result<Option<Season>> {
        for s in &self.sources {
            match s.season(number) {
                Ok(Some(v)) => return Ok(Some(v)),
                Ok(None) => continue,
                Err(e) => warn!(source = s.name(), %number, error = %e, "season fetch failed"),
            }
        }
        Ok(None)
    }

    fn episode(&self, arc_normalized: &str, episode_number: u32) -> Result<Option<Episode>> {
        for s in &self.sources {
            match s.episode(arc_normalized, episode_number) {
                Ok(Some(v)) => return Ok(Some(v)),
                Ok(None) => continue,
                Err(e) => warn!(
                    source = s.name(),
                    arc = %arc_normalized,
                    ep = episode_number,
                    error = %e,
                    "episode fetch failed"
                ),
            }
        }
        Ok(None)
    }

    fn image(&self, kind: ImageKind) -> Result<Option<Vec<u8>>> {
        for s in &self.sources {
            match s.image(kind) {
                Ok(Some(v)) => return Ok(Some(v)),
                Ok(None) => continue,
                Err(e) => warn!(source = s.name(), error = %e, "image fetch failed"),
            }
        }
        Ok(None)
    }
}
