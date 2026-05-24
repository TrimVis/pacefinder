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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_video_accepts_known_extensions() {
        for name in ["show.mkv", "movie.mp4", "file.m4v", "old.avi"] {
            assert!(is_video(Path::new(name)), "{name} should be video");
        }
    }

    #[test]
    fn is_video_case_insensitive() {
        assert!(is_video(Path::new("SHOW.MKV")));
        assert!(is_video(Path::new("Movie.Mp4")));
    }

    #[test]
    fn is_video_rejects_non_video_extensions() {
        for name in ["doc.txt", "image.jpg", "subs.srt", "show.webm"] {
            assert!(!is_video(Path::new(name)), "{name} should not be video");
        }
    }

    #[test]
    fn is_video_rejects_files_without_extension() {
        assert!(!is_video(Path::new("README")));
        // Leading-dot file has no Path-extension by Rust semantics.
        assert!(!is_video(Path::new(".mkv")));
    }

    #[test]
    fn is_video_treats_part_suffix_as_real_extension() {
        // Right behavior: .mkv.part is a download-in-progress — skip it.
        assert!(!is_video(Path::new("show.mkv.part")));
    }
}
