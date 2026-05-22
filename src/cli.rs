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
}
