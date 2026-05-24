use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::debug;
use walkdir::WalkDir;

use crate::fs_util::canonicalize_root;

pub(crate) const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "m4v", "avi"];

pub fn run(root: &Path) -> Result<()> {
    let root = canonicalize_root(root)?;
    debug!(path = %root.display(), "scanning library");

    let mut found: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(&root).follow_links(false) {
        let entry = entry.context("walking directory")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.into_path();
        if is_video(&path) {
            debug!(file = %path.display(), "found video");
            found.push(path);
        }
    }

    found.sort();
    for path in &found {
        let rel = path.strip_prefix(&root).unwrap_or(path);
        println!("{}", rel.display());
    }
    debug!(count = found.len(), "scan complete");
    Ok(())
}

pub(crate) fn is_video(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        let lower = e.to_ascii_lowercase();
        VIDEO_EXTS.contains(&lower.as_str())
    })
}
