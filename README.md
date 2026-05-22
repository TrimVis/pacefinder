# PaceFinder

A CLI that generates Kodi-format NFO sidecar files and artwork for a
[One Pace](https://onepace.net) fan-cut media library, so Jellyfin, Plex, and
Kodi can present the arcs as proper seasons with titles, descriptions, and
posters.

## Status

`pacefinder generate <series-root>` writes valid Kodi NFOs (`tvshow.nfo`,
`season.nfo`, per-episode `.nfo`) verified end-to-end against Jellyfin
10.11.9: series, seasons, and episodes ingest with the correct titles,
plots, original-title, and S/E numbering. Image fetching is not yet
implemented.

## Why

The previous community plugin (`jwueller/jellyfin-plugin-onepace`) relied on
the One Pace GraphQL API, which was retired when the site was rebuilt in early
2026. PaceFinder takes a different approach: a media-server-agnostic CLI that
emits NFO files, pulling its metadata from a pluggable `DataSource`. The first
data source wraps the community-maintained
[SpykerNZ/one-pace-for-plex](https://github.com/SpykerNZ/one-pace-for-plex)
dataset.

## Prerequisites

- Rust stable (1.85+, for edition 2024) — `rustup` will pick this up from
  `rust-toolchain.toml` automatically.
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
cargo run -- scan /path/to/onepace/library

# generate NFOs for every recognized file
cargo run -- generate "/path/to/library/One Pace"
cargo run -- generate "/path/to/library/One Pace" --dry-run
```

Available subcommands:

| Command | Description |
|---|---|
| `scan <path>` | Walk the path and list video files that would be processed. |
| `generate <series-root>` | Fetch metadata via SpykerNZ and write Kodi NFOs next to every recognized file. |

`generate` flags: `--dry-run`, `--refresh` (bypass HTTP cache),
`--cache-ttl-hours N` (default 24).

Global flags:

- `--log <directive>` — `tracing-subscriber` env filter. Defaults to `info`.
  Also reads `PACEFINDER_LOG`.

## Required library layout

Jellyfin (and Plex, Kodi) treat every immediate child of a TV library as a
separate Series. For NFOs to be associated with one *One Pace* series, the
path you pass to `pacefinder generate` must itself be the series folder, with
arc folders directly inside:

```
<jellyfin-library-root>/
  One Pace/                                ← pass this to `pacefinder generate`
    tvshow.nfo                             ← written by pacefinder
    [One Pace][1-7] Romance Dawn [1080p]/  ← arc = season
      season.nfo                           ← written by pacefinder
      [One Pace][1] Romance Dawn 01 [1080p][D767799C].mkv
      [One Pace][1] Romance Dawn 01 [1080p][D767799C].nfo  ← written by pacefinder
      ...
    [One Pace][8-21] Orange Town [1080p]/
      ...
```

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
cargo run -- scan ../testlib              # generate / refresh NFOs
docker compose restart jellyfin           # or trigger a library scan in the UI
```

To wipe Jellyfin state for a clean test:

```sh
docker compose down
rm -rf docker/jellyfin-config docker/jellyfin-cache
```

## Project layout

```
src/
  main.rs        entry, tokio runtime, dispatch
  cli.rs         clap argument structs
  scan.rs        directory walking
docker/
  compose.yaml   Jellyfin 10.11 test harness
testlib/         (gitignored) sample One Pace media for local dev
```

## License

MIT — see [LICENSE](LICENSE).
