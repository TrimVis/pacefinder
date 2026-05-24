//! Domain types for One Pace metadata.
//!
//! These are serde-friendly so adapters can deserialize directly from
//! upstream representations and NFO writers can serialize without an
//! intermediate hop.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Series {
    pub title: String,
    pub showtitle: String,
    pub original_title: Option<String>,
    pub plot: String,
    pub named_seasons: Vec<NamedSeason>,
    /// Kodi/Jellyfin display-order hint written into tvshow.nfo (e.g.
    /// "absolute", "aired", "dvd"). `None` lets the media server use its
    /// default ("aired" for Jellyfin).
    pub display_order: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamedSeason {
    pub number: u32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Season {
    pub number: u32,
    pub title: String,
    pub plot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub showtitle: String,
    pub season: u32,
    pub number: u32,
    pub title: String,
    pub plot: Option<String>,
    pub premiered: Option<String>,
    pub aired: Option<String>,
}
