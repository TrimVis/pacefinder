use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "pacefinder",
    version,
    about = "Generate Kodi-format NFOs and artwork for a One Pace media library."
)]
pub struct Cli {
    #[arg(long, global = true, env = "PACEFINDER_LOG", default_value = "info")]
    pub log: String,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Walk a media directory and report what would be processed.
    Scan {
        /// Root of the One Pace library to scan.
        path: PathBuf,
    },
    /// Generate NFO sidecars for every recognized One Pace file in `path`.
    Generate {
        /// Root of the One Pace library.
        path: PathBuf,

        /// Resolve and log writes without touching the filesystem.
        #[arg(long)]
        dry_run: bool,

        /// Cache TTL in hours for upstream metadata fetches.
        #[arg(long, default_value_t = 24)]
        cache_ttl_hours: u64,

        /// Bypass the on-disk HTTP cache.
        #[arg(long)]
        refresh: bool,
    },
}
