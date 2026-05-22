//! `reorder` subcommand: wrap top-level arc folders inside a series folder
//! so the library matches the layout Jellyfin/Plex/Kodi expect.
//!
//! Idempotent. Loose files at the top level (not in an arc folder) are
//! flagged but not moved; reconstructing their arc-folder name requires
//! inspecting every episode's chapter range and is left for a later pass.

use anyhow::{Context, Result, anyhow};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::matcher::{ParsedFile, is_arc_folder_name};

pub struct Options {
    pub dry_run: bool,
    pub series_folder: String,
}

pub fn run(root: &Path, opts: Options) -> Result<()> {
    let root = root.canonicalize().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            anyhow!("path does not exist: {}", root.display())
        } else {
            anyhow!("{}: {}", root.display(), e)
        }
    })?;
    let target = root.join(&opts.series_folder);
    info!(
        path = %root.display(),
        series_folder = %opts.series_folder,
        dry_run = opts.dry_run,
        "reordering library"
    );

    let mut to_move: Vec<PathBuf> = Vec::new();
    let mut loose_files: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let ft = entry.file_type()?;

        if ft.is_dir() {
            if name == opts.series_folder {
                continue;
            }
            if is_arc_folder_name(name) {
                to_move.push(path);
            }
        } else if ft.is_file() && ParsedFile::from_path(&path).is_some() {
            loose_files.push(path);
        }
    }

    if !loose_files.is_empty() {
        warn!(
            "{} loose One Pace files at library root will not be moved \
             (file-level grouping is unimplemented):",
            loose_files.len()
        );
        for p in &loose_files {
            warn!("  {}", p.display());
        }
    }

    if to_move.is_empty() {
        info!("no arc folders to move — already organized");
        return Ok(());
    }

    if !opts.dry_run {
        fs::create_dir_all(&target).with_context(|| format!("creating {}", target.display()))?;
    }

    let mut moved = 0usize;
    let mut skipped = 0usize;
    for source in &to_move {
        let name = source.file_name().expect("dir entry has name");
        let dest = target.join(name);
        if dest.exists() {
            warn!(source = %source.display(), dest = %dest.display(), "destination exists, skipping");
            skipped += 1;
            continue;
        }
        if opts.dry_run {
            info!(would_move = %source.display(), to = %dest.display(), "[dry-run]");
        } else {
            fs::rename(source, &dest)
                .with_context(|| format!("moving {} -> {}", source.display(), dest.display()))?;
            info!(from = %source.display(), to = %dest.display(), "moved");
        }
        moved += 1;
    }

    info!(moved, skipped, total = to_move.len(), "reorder complete");
    Ok(())
}
