use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "pacefinder",
    version,
    about = "Generate Kodi-format NFOs and artwork for a One Pace media library"
)]
pub struct Cli {
    /// tracing-subscriber env filter. CLI takes precedence over the env
    /// var, env var takes precedence over the default.
    #[arg(long, global = true, env = "PACEFINDER_LOG", default_value = "info")]
    pub log: String,

    #[command(subcommand)]
    pub command: Command,
}

// Order here is the order in `--help`. Put the primary user-facing command
// first, then the diagnostic, then the one-time setup helper, then the
// utilities.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Generate NFO sidecars for every recognized One Pace file under <path>
    Generate {
        /// Root of the One Pace series folder (one level above the arc folders).
        path: PathBuf,

        /// Resolve and log writes without touching the filesystem.
        #[arg(long)]
        dry_run: bool,

        /// Cache TTL in hours for upstream metadata fetches.
        #[arg(long, default_value_t = 168)]
        cache_ttl_hours: u64,

        /// Bypass the on-disk HTTP cache.
        #[arg(long)]
        refresh: bool,
    },
    /// Walk a media directory and report what was recognized
    Scan {
        /// Root of the One Pace library to scan.
        path: PathBuf,
    },
    /// Wrap top-level arc folders inside a series folder
    Reorder {
        /// Library root containing the arc folders.
        path: PathBuf,

        /// Name of the series wrapper folder.
        #[arg(long, default_value = "One Pace")]
        series_folder: String,

        /// Resolve and log moves without touching the filesystem.
        #[arg(long)]
        dry_run: bool,
    },
    /// Print version and exit
    Version,
}
