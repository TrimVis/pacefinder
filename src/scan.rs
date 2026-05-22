use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use walkdir::WalkDir;

const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "m4v", "avi"];

pub fn run(root: &Path) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;
    info!(path = %root.display(), "scanning library");

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
    info!(count = found.len(), "scan complete");
    Ok(())
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            VIDEO_EXTS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}
