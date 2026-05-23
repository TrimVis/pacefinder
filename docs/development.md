# Development

## Build

```sh
cargo build              # debug
cargo build --release    # release binary at ./target/release/pacefinder
cargo clippy -- -D warnings
cargo fmt --check
cargo test
```

Rust stable 1.85+ (for edition 2024). `rustup` picks the right channel up
from `rust-toolchain.toml` automatically.

## Dev loop with Jellyfin 10.11

The repo ships a Docker Compose harness so you can verify generated NFOs
are consumed correctly by a real Jellyfin server.

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

After `generate` runs, Jellyfin won't auto-refresh items it has already
indexed. See [troubleshooting](troubleshooting.md#my-re-run-didnt-update-the-metadata-in-jellyfin)
for the per-item refresh trick.

## Project layout

```
src/
  main.rs         entry, tracing setup, dispatch
  cli.rs          clap argument structs
  generate.rs     `generate` subcommand
  reorder.rs      `reorder` subcommand
  cleanup.rs      `cleanup` subcommand (rmdir empty / .ignore foreign)
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
    sheet.rs      Google Sheet adapter (CRC oracle + episode synthesis)
docker/
  compose.yaml    Jellyfin 10.11 test harness
docs/             user + developer docs (this file, troubleshooting, data sources)
testlib/          (gitignored) sample One Pace media for local dev
```

## Releases

Tag pushes (`v*.*.*`) trigger `.github/workflows/release.yml`, which
cross-builds for Linux (x86_64 / aarch64 musl), macOS (Intel / Apple
Silicon), and Windows (x86_64 MSVC), uploads each archive plus a sibling
`.sha256`, and rewrites the release body with the install snippet from
`.github/install-snippet.md`.

The install snippet is the single source of truth — duplicated (carefully)
between that file and the README so the file can be rendered with the tag
substituted at release time. Keep them in sync; markers in the README
(`<!-- install-snippet:start -->` / `<!-- install-snippet:end -->`) call
out the section to mirror.
