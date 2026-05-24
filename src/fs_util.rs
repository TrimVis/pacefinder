use anyhow::{Result, anyhow};
use std::io;
use std::path::{Path, PathBuf};

/// Canonicalize a user-supplied library path with a friendlier error than
/// std's default. NotFound becomes a path-shaped message instead of the
/// raw `No such file or directory (os error 2)`.
pub fn canonicalize_root(root: &Path) -> Result<PathBuf> {
    root.canonicalize().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            anyhow!("path does not exist: {}", root.display())
        } else {
            anyhow!("{}: {}", root.display(), e)
        }
    })
}
