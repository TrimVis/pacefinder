# Data sources

PaceFinder pulls metadata from multiple upstream sources through a pluggable
`DataSource` trait. Sources are tried in order; the first to return data
wins per field. The chain is configured in `src/source/mod.rs::default_chain`
— a one-line change to add or reorder.

## The default chain

1. **onepace.net** — current canonical arc list (38 arcs incl. specials).
   Provides season titles and fresh descriptions. Episode-level data is
   not exposed by the site. Fetched via the `/watch` RSC payload.
2. **[SpykerNZ/one-pace-for-plex](https://github.com/SpykerNZ/one-pace-for-plex)**
   — hand-maintained NFO bundle. Provides series metadata, per-episode
   titles/plots/airdates, and poster artwork. Last updated Jan 2024, so
   newer arcs fall through to the other sources.
3. **A community-tracked [One Pace Google Sheet](https://docs.google.com/spreadsheets/d/1HQRMJgu_zArp-sLnvFMDzOyjdsht87eFLECxMK858lA/edit?gid=0)**
   — per-arc episode lists keyed by MKV CRC32. We use it two ways:
   - **CRC oracle:** when a file's CRC matches a sheet entry, override
     filename-derived arc + episode with the sheet's authoritative mapping.
   - **Episode synthesis:** when SpykerNZ has no rich data for an arc,
     synthesize minimal `Episode` records from its chapter / anime-episode
     / release-date columns.

## Coverage caveat — sheet CRCs

The sheet only tracks the *latest re-encode* of each episode. Older files
in your library fall through to filename-derived arc + episode-number
identification, which still works fine — CRC override is an enhancement,
not a requirement.

## Arc-name aliases

The community renames arcs over time and the same arc has multiple
spellings across sources. Aliases live in two small maps:

- `src/source/spykernz.rs::arc_alias` — user-side ↔ SpykerNZ spelling
  (e.g. "Whiskey Peak" → "Whisky Peak").
- `src/source/sheet.rs::arc_alias` — user-side ↔ sheet spelling (e.g.
  "Arabasta" → "Alabasta", "Wano Act 1" → "Wano").

Add new entries as you spot drift. They're plain `match` arms — one line
each, no infra changes needed.

## Adding a new source

Implement `crate::source::DataSource` for your adapter, expose a
constructor, and add it to the `Vec` in `src/source/mod.rs::default_chain`.
`identify_by_crc` has a default `Ok(None)` impl, so an adapter that only
provides series/season/episode metadata stays small. See
`src/source/onepacenet.rs` for a minimal example (season-only) and
`src/source/sheet.rs` for a richer one.
