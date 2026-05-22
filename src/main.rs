mod cli;
mod generate;
mod matcher;
mod model;
mod nfo;
mod reorder;
mod scan;
mod source;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let args = Cli::parse();

    // Compact, no timestamp, no module target — this is a CLI tool, not a
    // log-aggregator feed.
    let filter = EnvFilter::try_new(&args.log).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .without_time()
        .with_target(false)
        .compact()
        .init();

    match args.command {
        Command::Version => {
            println!("pacefinder {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Scan { path } => scan::run(&path),
        Command::Reorder {
            path,
            series_folder,
            dry_run,
        } => reorder::run(
            &path,
            reorder::Options {
                dry_run,
                series_folder,
            },
        ),
        Command::Generate {
            path,
            dry_run,
            cache_ttl_hours,
            refresh,
        } => generate::run(
            &path,
            generate::Options {
                dry_run,
                cache_ttl: Duration::from_secs(cache_ttl_hours * 3600),
                refresh,
            },
        ),
    }
}
