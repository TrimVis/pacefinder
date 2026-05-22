mod cli;
mod generate;
mod matcher;
mod model;
mod nfo;
mod scan;
mod source;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    let filter = EnvFilter::try_new(&args.log).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    match args.command {
        Command::Scan { path } => scan::run(&path),
        Command::Generate {
            path,
            dry_run,
            cache_ttl_hours,
            refresh,
        } => {
            generate::run(
                &path,
                generate::Options {
                    dry_run,
                    cache_ttl: Duration::from_secs(cache_ttl_hours * 3600),
                    refresh,
                },
            )
            .await
        }
    }
}
