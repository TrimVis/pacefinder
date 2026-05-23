# Data sources

PaceFinder pulls metadata from multiple upstream sources through a pluggable
`DataSource` trait. Sources are tried in order; the first to return data
wins per field. The chain isn't load-bearing — adding or removing a source
is a one-line change in `src/generate.rs::build_source`.

## Why this exists

The previous community plugin (`jwueller/jellyfin-plugin-onepace`) relied on
the One Pace GraphQL API, which was retired when the site was rebuilt in
early 2026. PaceFinder takes a different approach: a media-server-agnostic
CLI that emits NFO files, pulling metadata from several upstreams and
composing the results.

## The default chain

1. **onepace.net** — current canonical arc list (38 arcs incl. specials).
   Provides season titles and fresh descriptions. Episode-level data is
   not exposed by the site. Fetched via the `/watch` RSC payload.
2. **[SpykerNZ/one-pace-for-plex](https://github.com/SpykerNZ/one-pace-for-plex)**
   — hand-maintained NFO bundle. Provides series metadata, per-episode
   titles/plots/airdates, and poster artwork. Last updated Jan 2024, so
   newer arcs fall through to the other sources.
3. **[One Pace Google Sheet](https://docs.google.com/spreadsheets/d/1HQRMJgu_zArp-sLnvFMDzOyjdsht87eFLECxMK858lA/edit?gid=0)**
   *(provenance unverified — looks community-tracked, no claim of official
   One Pace endorsement)* — per-arc episode lists keyed by MKV CRC32. We
   use it two ways:
   - **CRC oracle:** when a file's CRC matches a sheet entry, override
     filename-derived arc + episode with the sheet's authoritative mapping.
   - **Episode synthesis:** when SpykerNZ has no rich data for an arc,
     synthesize minimal `Episode` records from its chapter / anime-episode
     / release-date columns.

## Coverage caveat — sheet CRCs

The sheet lists CRCs for the *latest re-encode* of each episode. Older
releases in your library will not match by CRC; they fall through to the
filename-derived arc + episode-number path (which still works fine — that's
the original behavior). If your library is mostly recent encodes, CRC
override fires often; if mostly older, it's mostly a no-op. Either way the
data still flows from SpykerNZ + sheet synthesis, so coverage doesn't
degrade.

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
constructor, and add it to the `Vec` in `src/generate.rs::build_source`.
The trait has default implementations for the rarely-used method
(`identify_by_crc`), so an adapter that only does one or two things stays
small. See `src/source/onepacenet.rs` for a minimal example (season-only)
and `src/source/sheet.rs` for a richer one.
