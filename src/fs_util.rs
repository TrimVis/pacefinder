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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_root_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let canon = canonicalize_root(dir.path()).unwrap();
        assert_eq!(canon, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn canonicalize_root_missing_path() {
        let p = PathBuf::from("/nonexistent_pacefinder_test_dir/whatever");
        let err = canonicalize_root(&p).unwrap_err().to_string();
        assert!(err.contains("path does not exist"), "got: {err}");
        assert!(err.contains("whatever"), "got: {err}");
    }

    #[test]
    fn canonicalize_root_accepts_files_too() {
        // Function is named *_root but doesn't enforce dir-ness; pinning
        // the current behavior so future refactors are deliberate.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        assert!(canonicalize_root(&file).is_ok());
    }
}
