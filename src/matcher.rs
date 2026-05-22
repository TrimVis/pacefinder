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

// Chapter ranges in the wild appear in several shapes:
//   [1]        single chapter
//   [1-7]      contiguous range
//   [42,22]    multi-chapter (comma)
//   [153-155, 142]   range + extra
//   [1058-]    open-ended (folder-only — no upper bound declared yet)
// Allow any combination of digits, commas, dashes, and whitespace inside.
const RANGE_BODY: &str = r"\d+[\d,\s-]*";

static FOLDER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?x)
        ^\[One\ Pace\]
        \[{RANGE_BODY}\]
        \s+.+?
        \s+\[[^\]]+\]$
        "
    ))
    .expect("static regex")
});

// Episode files where the arc is broken into numbered episodes:
//   [One Pace][<range>] <Arc> <ep> [<res>][<crc>].<ext>
static FILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?x)
        ^\[One\ Pace\]
        \[(?P<range>{RANGE_BODY})\]
        \s+(?P<arc>.+?)
        \s+(?P<ep>\d+)
        \s+\[(?P<res>[^\]]+)\]
        (?:\[(?P<crc>[0-9A-Fa-f]{{8}})\])?
        \.(?P<ext>[A-Za-z0-9]+)$
        "
    ))
    .expect("static regex")
});

// Single-file arcs where the entire season is one file with no episode number:
//   [One Pace][<range>] <Arc> [<res>][<crc>].<ext>
// Treated as episode 1 of the matching season.
static FILE_RE_SINGLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?x)
        ^\[One\ Pace\]
        \[(?P<range>{RANGE_BODY})\]
        \s+(?P<arc>.+?)
        \s+\[(?P<res>[^\]]+)\]
        (?:\[(?P<crc>[0-9A-Fa-f]{{8}})\])?
        \.(?P<ext>[A-Za-z0-9]+)$
        "
    ))
    .expect("static regex")
});

impl ParsedFile {
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        Self::from_filename(name)
    }

    pub fn from_filename(name: &str) -> Option<Self> {
        if let Some(caps) = FILE_RE.captures(name) {
            return Some(Self {
                arc: caps["arc"].trim().to_string(),
                episode: caps["ep"].parse().ok()?,
                chapter_range: caps["range"].to_string(),
                resolution: Some(caps["res"].to_string()),
                crc32: caps.name("crc").map(|m| m.as_str().to_ascii_uppercase()),
                extension: caps["ext"].to_string(),
            });
        }
        let caps = FILE_RE_SINGLE.captures(name)?;
        Some(Self {
            arc: caps["arc"].trim().to_string(),
            episode: 1,
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
    fn parses_multi_chapter_range_with_comma() {
        let p = ParsedFile::from_filename(
            "[One Pace][42,22] Gaimon 01 [1080p][0C2DBF75].mkv",
        )
        .unwrap();
        assert_eq!(p.chapter_range, "42,22");
        assert_eq!(p.arc, "Gaimon");
        assert_eq!(p.episode, 1);
    }

    #[test]
    fn parses_range_with_comma_and_space() {
        let p = ParsedFile::from_filename(
            "[One Pace][153-155, 142] Drum Island 08 [1080p][9794072D].mkv",
        )
        .unwrap();
        assert_eq!(p.arc, "Drum Island");
        assert_eq!(p.episode, 8);
    }

    #[test]
    fn parses_single_file_arc_without_episode_number() {
        let p = ParsedFile::from_filename(
            "[One Pace][35-75] The Adventures of Buggy's Crew [1080p][E75794DB].mkv",
        )
        .unwrap();
        assert_eq!(p.arc, "The Adventures of Buggy's Crew");
        assert_eq!(p.episode, 1);
        assert_eq!(p.chapter_range, "35-75");
    }

    #[test]
    fn arc_folder_regex_accepts_open_ended_range() {
        assert!(is_arc_folder_name("[One Pace][1058-] Egghead [1080p]"));
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
