//! Download subsystem.
//!
//! `dl` is sibling to `nfo` and `source` because fetching media is its own
//! concern — different upstream (BitTorrent), different output (queued
//! torrents, not on-disk files), different failure modes.
//!
//! Public surface:
//!
//! - [`Release`] — one downloadable item: a parsed filename + the magnet URI.
//! - [`parse_magnet`] — pull the display name + info-hash out of a magnet
//!   URI without taking a dep on a full magnet-link crate.
//! - [`qbittorrent::QbtClient`] — the only download backend today. A
//!   trait abstraction lands when there's a second client to slot in.

pub mod qbittorrent;

use crate::matcher::ParsedFile;

/// One downloadable episode/file from an upstream releases listing.
#[derive(Debug, Clone)]
pub struct Release {
    /// Raw magnet URI, ready to feed to a torrent client.
    pub magnet: String,
    /// Filename pulled from the magnet's `dn=` parameter. Carries arc,
    /// episode, resolution, CRC32 — the identification surface.
    pub filename: String,
    /// Parser output for `filename`. `None` means we recognized it as a
    /// magnet but the filename didn't match any One Pace pattern (legacy
    /// release naming, "Paced One Piece" variants, etc.). We skip those.
    pub parsed: Option<ParsedFile>,
}

impl Release {
    /// Pixel height parsed from the resolution token, e.g. `1080p` → 1080,
    /// `640x480 x265 AAC` → 480. Used by the download CLI to pick the
    /// best release at or below the user's `--resolution` cap.
    pub fn height(&self) -> Option<u32> {
        let res = self.parsed.as_ref()?.resolution.as_deref()?;
        height_of_resolution(res)
    }
}

// ---------- magnet parsing ----------

/// Just enough of a magnet URI to be useful: the BitTorrent info-hash
/// and the display name (URL-decoded). Trackers are passed through
/// untouched in the raw magnet string; we don't need them parsed.
#[derive(Debug, Clone)]
pub struct ParsedMagnet {
    pub btih: String,
    pub display_name: Option<String>,
}

/// Parse a `magnet:?…` URI into its useful components. Returns `None` if
/// the input isn't a magnet URI or lacks an info-hash. Doesn't validate
/// tracker URLs (some BT clients silently drop bad ones).
pub fn parse_magnet(uri: &str) -> Option<ParsedMagnet> {
    let body = uri.strip_prefix("magnet:?")?;

    let mut btih = None;
    let mut display_name = None;

    for pair in body.split('&') {
        let (k, v) = pair.split_once('=')?;
        match k {
            "xt" => {
                if let Some(hash) = v.strip_prefix("urn:btih:") {
                    btih = Some(hash.to_string());
                }
            }
            "dn" => display_name = Some(urldecode_plus(v)),
            _ => {}
        }
    }

    Some(ParsedMagnet {
        btih: btih?,
        display_name,
    })
}

/// Minimal `application/x-www-form-urlencoded`-style decoder: `+` → space,
/// `%XX` → byte. Enough for magnet `dn`/`tr` values. Doesn't validate
/// UTF-8; bad sequences become replacement characters via `String::from_utf8_lossy`.
fn urldecode_plus(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut iter = input.chars().peekable();
    while let Some(c) = iter.next() {
        match c {
            '+' => bytes.push(b' '),
            '%' => {
                let h1 = iter.next();
                let h2 = iter.next();
                match (
                    h1.and_then(|c| c.to_digit(16)),
                    h2.and_then(|c| c.to_digit(16)),
                ) {
                    (Some(a), Some(b)) => bytes.push(((a << 4) | b) as u8),
                    _ => {
                        // Malformed — preserve as literal.
                        bytes.push(b'%');
                        if let Some(c1) = h1 {
                            bytes.extend(c1.to_string().as_bytes());
                        }
                        if let Some(c2) = h2 {
                            bytes.extend(c2.to_string().as_bytes());
                        }
                    }
                }
            }
            c => bytes.extend(c.to_string().as_bytes()),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// "1080p" → 1080, "720p" → 720, "640x480 x265 AAC" → 480,
/// "WEBRip 1080" → 1080. Pulls the highest 3-4 digit number that looks
/// like a vertical resolution. Returns `None` if no plausible number.
fn height_of_resolution(res: &str) -> Option<u32> {
    // Quick path: trailing "p" like "1080p".
    if let Some(num) = res.strip_suffix('p').and_then(|n| n.parse::<u32>().ok()) {
        return Some(num);
    }
    // Otherwise: find the smallest two numbers in "WxH" if present, take H.
    if let Some((_, after_x)) = res.split_once(['x', 'X']) {
        // Grab the run of digits at the start.
        let digits: String = after_x.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(h) = digits.parse::<u32>() {
            return Some(h);
        }
    }
    // Last resort: any 3-4 digit number in the string.
    let mut best = None;
    let bytes = res.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let s = &res[i..j];
            if (3..=4).contains(&s.len())
                && let Ok(n) = s.parse::<u32>()
                && (240..=2160).contains(&n)
            {
                best = Some(best.map_or(n, |cur: u32| cur.max(n)));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_one_pace_magnet() {
        let uri = "magnet:?xt=urn:btih:73e0d240e57bf1143a5684a654eee46275a13ef5\
                   &dn=%5BOne+Pace%5D%5B123-126%5D+Little+Garden+04+%5B1080p%5D%5BCA509241%5D.mkv\
                   &tr=http%3A%2F%2Fnyaa.tracker.wf%3A7777%2Fannounce";
        let m = parse_magnet(uri).unwrap();
        assert_eq!(m.btih, "73e0d240e57bf1143a5684a654eee46275a13ef5");
        assert_eq!(
            m.display_name.as_deref(),
            Some("[One Pace][123-126] Little Garden 04 [1080p][CA509241].mkv"),
        );
    }

    #[test]
    fn rejects_non_magnet_uri() {
        assert!(parse_magnet("https://example.com/foo").is_none());
        assert!(parse_magnet("magnet:?dn=foo").is_none()); // no xt= info-hash
    }

    #[test]
    fn height_handles_common_tokens() {
        assert_eq!(height_of_resolution("1080p"), Some(1080));
        assert_eq!(height_of_resolution("720p"), Some(720));
        assert_eq!(height_of_resolution("480p"), Some(480));
        assert_eq!(height_of_resolution("640x480 x265 AAC"), Some(480));
        assert_eq!(height_of_resolution("nonsense"), None);
    }

    #[test]
    fn release_height_routes_through_resolution() {
        let r = Release {
            magnet: "magnet:?xt=urn:btih:abc".into(),
            filename: "x.mkv".into(),
            parsed: Some(ParsedFile {
                arc: "x".into(),
                episode: 1,
                crc32: None,
                resolution: Some("720p".into()),
            }),
        };
        assert_eq!(r.height(), Some(720));
    }
}
