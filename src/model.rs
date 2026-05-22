#![allow(dead_code)] // ImageKind is wired up by step 13
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageKind {
    SeriesPoster,
    SeasonPoster { number: u32 },
}
