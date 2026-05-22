# PaceFinder

**Run `pacefinder generate /path/to/your/library/One\ Pace` and your
[One Pace](https://onepace.net) fan-cut files will show up as proper
seasons in Jellyfin, Plex, and Kodi — correct titles, plots, posters,
S/E numbering.**

PaceFinder walks a One Pace media library, matches each file to its arc
and episode, fetches metadata from multiple upstream sources, and writes
Kodi-format NFO sidecars + posters next to your media.

## Status

`pacefinder generate <series-root>` writes valid Kodi NFOs (`tvshow.nfo`,
`season.nfo`, per-episode `.nfo`) and downloads series + season poster art.
Verified end-to-end against Jellyfin 10.11: series, seasons, and episodes
ingest with correct titles, plots, original-title, S/E numbering, and
posters.

## Why

The previous community plugin (`jwueller/jellyfin-plugin-onepace`) relied on
the One Pace GraphQL API, which was retired when the site was rebuilt in early
2026. PaceFinder takes a different approach: a media-server-agnostic CLI that
emits NFO files, pulling its metadata from a pluggable `DataSource`.

Multiple sources are tried in order, with the first to return data winning
per field. Default chain:

1. **onepace.net** — current canonical arc list (38 arcs incl. specials).
   Provides season titles and fresh descriptions. Episode-level data is
   not exposed by the site. Fetched via the `/watch` RSC payload.
2. **[SpykerNZ/one-pace-for-plex](https://github.com/SpykerNZ/one-pace-for-plex)**
   — hand-maintained NFO bundle. Provides series metadata, per-episode
   titles/plots/airdates, and poster artwork. Last updated Jan 2024, so it
   does not cover the Egghead split or newer arcs — those fall through to
   onepace.net.

Known additional source not yet wired: the
[official One Pace Google Sheet](https://docs.google.com/spreadsheets/d/1HQRMJgu_zArp-sLnvFMDzOyjdsht87eFLECxMK858lA/edit?gid=0)
exposes the arc list with chapter ranges and pace-episode counts. Adding it
as a fallback is straightforward.

## Install

- Rust stable (1.85+, for edition 2024) — `rustup` will pick this up from
  `rust-toolchain.toml` automatically.
- `cargo install --path .` or grab a release binary (see Releases on GitHub).
- Docker + Docker Compose v2 (only required for the integration harness).

## Build

```sh
cargo build              # debug
cargo build --release    # release binary at ./target/release/pacefinder
cargo clippy -- -D warnings
cargo fmt --check
```

## Run

```sh
# walk a directory and list recognized One Pace files
pacefinder scan /path/to/onepace/library

# generate NFOs + posters for every recognized file
pacefinder generate "/path/to/library/One Pace"
pacefinder generate "/path/to/library/One Pace" --dry-run

# wrap top-level arc folders into a series folder if your layout is flat
pacefinder reorder /path/to/library --dry-run

# inspect or wipe the metadata cache
pacefinder cache path
pacefinder cache clear
```

### Subcommands

| Command | Description |
|---|---|
| `generate <series-root>` | Fetch metadata and write NFOs + posters next to every recognized file. |
| `scan <path>` | Walk the path and list video files; useful diagnostic. |
| `reorder <path>` | Wrap top-level arc folders inside a series folder (one-time setup if your layout is flat). |
| `cache path` / `cache clear` | Show where cached upstream responses live, or wipe them. |
| `version` | Print version. |

### Flags

| Flag | Description |
|---|---|
| `-v` / `-vv` | More verbose: debug / trace. |
| `-q` / `-qq` / `-qqq` | Less verbose: warn only / error only / silent. |
| `--log <directive>` | Power-user escape: `tracing-subscriber` env filter; also `PACEFINDER_LOG`. Overrides `-v`/`-q`. |
| `generate --dry-run` | Resolve and log writes without touching the filesystem. |
| `generate --refresh` | Bypass the on-disk HTTP cache. |
| `generate --cache-ttl 7d` | Cache TTL (humantime: `7d`, `24h`, `30m`). Default `7d`. |

## Required library layout

Jellyfin (and Plex, Kodi) treat every immediate child of a TV library as a
separate Series. For NFOs to be associated with one *One Pace* series, the
path you pass to `pacefinder generate` must itself be the series folder, with
arc folders directly inside:

```
<jellyfin-library-root>/
  One Pace/                                ← pass this to `pacefinder generate`
    tvshow.nfo                             ← written by pacefinder
    poster.png                             ← written by pacefinder
    [One Pace][1-7] Romance Dawn [1080p]/  ← arc = season
      season.nfo                           ← written by pacefinder
      poster.png                           ← written by pacefinder
      [One Pace][1] Romance Dawn 01 [1080p][D767799C].mkv
      [One Pace][1] Romance Dawn 01 [1080p][D767799C].nfo  ← written by pacefinder
      ...
    [One Pace][8-21] Orange Town [1080p]/
      ...
```

If your library is flat (arc folders at the top with no `One Pace/` wrapper),
run `pacefinder reorder <library>` first. PaceFinder warns when it detects
this layout to prevent the silent-failure case where each arc becomes its own
Jellyfin series.

Then in Jellyfin, add `<jellyfin-library-root>/` as a TV Shows library with
the "Nfo" local metadata reader enabled and remote metadata fetchers off.

## Dev loop with Jellyfin 10.11

The repo ships a Docker Compose harness so you can verify generated NFOs are
consumed correctly by a real Jellyfin server.

```sh
# one-time: drop a few sample files into ./testlib/
mkdir -p testlib
cp -r /path/to/some/onepace/arcs/* testlib/

# start jellyfin (first run pulls the image, ~500MB)
cd docker && docker compose up -d

# open http://127.0.0.1:8096 and complete the setup wizard
# add /media as a TV Shows library
```

The Jellyfin container bind-mounts:

- `./testlib` → `/media` (read-only, the library)
- `./docker/jellyfin-config` → `/config` (server state, plugins)
- `./docker/jellyfin-cache` → `/cache` (transcode cache)

To iterate:

```sh
cargo run -- generate "./testlib/One Pace"   # write NFOs + posters
docker compose restart jellyfin              # or trigger a library scan in the UI
```

To wipe Jellyfin state for a clean test:

```sh
docker compose down
rm -rf docker/jellyfin-config docker/jellyfin-cache
```

## Project layout

```
src/
  main.rs         entry, tracing setup, dispatch
  cli.rs          clap argument structs
  generate.rs     `generate` subcommand
  reorder.rs      `reorder` subcommand
  scan.rs         `scan` subcommand + shared video-extension filter
  matcher.rs      filename → ParsedFile + arc-name normalization
  model.rs        domain types (Series, Season, Episode, NamedSeason)
  nfo/
    kodi.rs       Kodi NFO XML shapes (parse + serialize)
    writer.rs     NFO write helpers (series, season, episode)
  source/
    mod.rs        DataSource trait, ImageKind
    cache.rs      on-disk HTTP cache (ureq + sha256-keyed)
    composite.rs  fallthrough source chain
    onepacenet.rs onepace.net /watch RSC adapter
    spykernz.rs   SpykerNZ GitHub-blob adapter
docker/
  compose.yaml    Jellyfin 10.11 test harness
testlib/          (gitignored) sample One Pace media for local dev
```

## License

MIT — see [LICENSE](LICENSE).
