#![allow(dead_code)] // removed once `generate` subcommand wires these in
//! Pluggable metadata data sources.
//!
//! A `DataSource` knows how to answer queries about One Pace's series,
//! seasons, episodes, and images. The first implementation wraps the
//! community-maintained SpykerNZ Plex dataset; future adapters can live
//! in this module or in external crates that implement the trait.

use anyhow::Result;
use async_trait::async_trait;

use crate::model::{Episode, ImageKind, Season, Series};

pub mod cache;
pub mod spykernz;

#[async_trait]
pub trait DataSource: Send + Sync {
    /// Human-readable adapter name, useful in logs.
    fn name(&self) -> &'static str;

    /// Series-level metadata (titles, plot, named-season map).
    async fn series(&self) -> Result<Series>;

    /// Per-season metadata. Returns `Ok(None)` if the source has no record
    /// of the requested season number.
    async fn season(&self, number: u32) -> Result<Option<Season>>;

    /// Episode metadata keyed by normalized arc name + 1-based episode number.
    /// Returns `Ok(None)` if the source has no record of that episode.
    async fn episode(
        &self,
        arc_normalized: &str,
        episode_number: u32,
    ) -> Result<Option<Episode>>;

    /// Image bytes for the given kind, if available.
    async fn image(&self, kind: ImageKind) -> Result<Option<Vec<u8>>>;
}
