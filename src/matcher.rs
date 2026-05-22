//! Parse One Pace release filenames.
//!
//! Episode files follow: `[One Pace][<chapter-range>] <Arc Name> <ep#> [<res>][<CRC>].ext`
//! Arc folders follow:   `[One Pace][<chapter-range>] <Arc Name> [<res>]`

use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFile {
    pub arc: String,
    pub episode: u32,
    pub chapter_range: String,
    pub resolution: Option<String>,
    pub crc32: Option<String>,
    pub extension: String,
}

/// True if `name` looks like an arc folder: `[One Pace][<range>] <Arc> [<res>]`.
pub fn is_arc_folder_name(name: &str) -> bool {
    FOLDER_RE.is_match(name)
}

static FOLDER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        ^\[One\ Pace\]
        \[\d+(?:-\d+)?\]
        \s+.+?
        \s+\[[^\]]+\]$
        ",
    )
    .expect("static regex")
});

static FILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        ^\[One\ Pace\]
        \[(?P<range>\d+(?:-\d+)?)\]
        \s+(?P<arc>.+?)
        \s+(?P<ep>\d+)
        \s+\[(?P<res>[^\]]+)\]
        (?:\[(?P<crc>[0-9A-Fa-f]{8})\])?
        \.(?P<ext>[A-Za-z0-9]+)$
        ",
    )
    .expect("static regex")
});

impl ParsedFile {
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        Self::from_filename(name)
    }

    pub fn from_filename(name: &str) -> Option<Self> {
        let caps = FILE_RE.captures(name)?;
        Some(Self {
            arc: caps["arc"].trim().to_string(),
            episode: caps["ep"].parse().ok()?,
            chapter_range: caps["range"].to_string(),
            resolution: Some(caps["res"].to_string()),
            crc32: caps.name("crc").map(|m| m.as_str().to_ascii_uppercase()),
            extension: caps["ext"].to_string(),
        })
    }
}

/// Normalize an arc name for matching: lowercase, collapse whitespace,
/// strip punctuation that varies between sources (commas, apostrophes).
pub fn normalize_arc(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_space = true;
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_episode_filename() {
        let p = ParsedFile::from_filename(
            "[One Pace][1] Romance Dawn 01 [1080p][D767799C].mkv",
        )
        .unwrap();
        assert_eq!(p.arc, "Romance Dawn");
        assert_eq!(p.episode, 1);
        assert_eq!(p.chapter_range, "1");
        assert_eq!(p.resolution.as_deref(), Some("1080p"));
        assert_eq!(p.crc32.as_deref(), Some("D767799C"));
        assert_eq!(p.extension, "mkv");
    }

    #[test]
    fn parses_chapter_range() {
        let p = ParsedFile::from_filename(
            "[One Pace][2-3] Romance Dawn 02 [1080p][ABCD1234].mkv",
        )
        .unwrap();
        assert_eq!(p.chapter_range, "2-3");
        assert_eq!(p.episode, 2);
    }

    #[test]
    fn parses_multi_word_arc_with_punctuation() {
        let p = ParsedFile::from_filename(
            "[One Pace][1051-1053] Long Ring Long Land 04 [720p][12345678].mkv",
        )
        .unwrap();
        assert_eq!(p.arc, "Long Ring Long Land");
        assert_eq!(p.episode, 4);
    }

    #[test]
    fn parses_filename_without_crc() {
        let p = ParsedFile::from_filename(
            "[One Pace][1] Romance Dawn 01 [1080p].mkv",
        )
        .unwrap();
        assert_eq!(p.crc32, None);
        assert_eq!(p.episode, 1);
    }

    #[test]
    fn parses_uppercase_crc_consistently() {
        let p = ParsedFile::from_filename(
            "[One Pace][1] Romance Dawn 01 [1080p][d767799c].mkv",
        )
        .unwrap();
        assert_eq!(p.crc32.as_deref(), Some("D767799C"));
    }

    #[test]
    fn rejects_non_one_pace_file() {
        assert!(ParsedFile::from_filename("Some.Other.Show.S01E01.mkv").is_none());
    }

    #[test]
    fn recognizes_arc_folder_names() {
        assert!(is_arc_folder_name("[One Pace][1-7] Romance Dawn [1080p]"));
        assert!(is_arc_folder_name("[One Pace][1051-1053] Wano [720p]"));
        assert!(!is_arc_folder_name(
            "[One Pace][1] Romance Dawn 01 [1080p][D767799C].mkv"
        ));
        assert!(!is_arc_folder_name("One Pace"));
        assert!(!is_arc_folder_name("Season 1"));
    }

    #[test]
    fn normalize_arc_strips_punctuation_and_lowercases() {
        assert_eq!(normalize_arc("Romance Dawn"), "romance dawn");
        assert_eq!(
            normalize_arc("The Adventures of Buggy's Crew"),
            "the adventures of buggy s crew"
        );
        assert_eq!(
            normalize_arc("If You Could Go Anywhere... The Adventures of the Straw Hats"),
            "if you could go anywhere the adventures of the straw hats"
        );
    }
}
