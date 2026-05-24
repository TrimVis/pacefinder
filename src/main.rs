mod cleanup;
mod cli;
mod dl;
mod download;
mod fs_util;
mod generate;
mod matcher;
mod model;
mod nfo;
mod reorder;
mod scan;
mod source;

use anyhow::Result;
use clap::Parser;
use cli::{CacheAction, Cli, Command};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let args = Cli::parse();

    // Precedence: --log (or PACEFINDER_LOG env, same flag) wins. Else map
    // -v/-q counts to a level. Else default to info (we want the user to
    // see progress like "wrote SXXEYY").
    let filter = if let Some(spec) = &args.log {
        EnvFilter::try_new(spec).unwrap_or_else(|_| EnvFilter::new("info"))
    } else {
        EnvFilter::new(level_from_counts(args.verbose, args.quiet))
    };
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
        Command::Cache { action } => match action {
            CacheAction::Path => {
                println!("{}", source::cache::cache_dir()?.display());
                Ok(())
            }
            CacheAction::Clear => source::cache::clear(),
        },
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
        Command::Cleanup {
            path,
            dry_run,
            remove,
        } => cleanup::run(&path, cleanup::Options { dry_run, remove }),
        Command::Generate {
            path,
            dry_run,
            cache_ttl,
            refresh,
            force,
            non_interactive,
            display_order,
            lock,
        } => generate::run(
            &path,
            generate::Options {
                dry_run,
                cache_ttl,
                refresh,
                force,
                non_interactive,
                display_order: display_order.as_kodi().to_string(),
                lock,
            },
        ),
        Command::Download {
            path,
            qbt_url,
            qbt_user,
            qbt_pass,
            qbt_category,
            resolution,
            cache_ttl,
            refresh,
            dry_run,
            prepopulate_nfo,
            requeue_existing,
            prefer_extended,
            only_arc,
            save_path_map,
            fail_on_empty,
        } => download::run(
            &path,
            download::Options {
                qbt_url,
                qbt_user,
                qbt_pass,
                qbt_category,
                resolution,
                cache_ttl,
                refresh,
                dry_run,
                prepopulate_nfo,
                requeue_existing,
                prefer_extended,
                only_arc,
                save_path_map,
                fail_on_empty,
            },
        ),
    }
}

fn level_from_counts(verbose: u8, quiet: u8) -> &'static str {
    match (verbose, quiet) {
        (_, 3..) => "off",
        (_, 2) => "error",
        (_, 1) => "warn",
        (0, _) => "info",
        (1, _) => "debug",
        (2.., _) => "trace",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_from_counts_default_info() {
        assert_eq!(level_from_counts(0, 0), "info");
    }

    #[test]
    fn level_from_counts_verbose_ladder() {
        assert_eq!(level_from_counts(1, 0), "debug");
        assert_eq!(level_from_counts(2, 0), "trace");
        assert_eq!(level_from_counts(5, 0), "trace");
    }

    #[test]
    fn level_from_counts_quiet_ladder() {
        assert_eq!(level_from_counts(0, 1), "warn");
        assert_eq!(level_from_counts(0, 2), "error");
        assert_eq!(level_from_counts(0, 3), "off");
        assert_eq!(level_from_counts(0, 99), "off");
    }
}
