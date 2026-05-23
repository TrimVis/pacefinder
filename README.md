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

3. **[Official One Pace Google Sheet](https://docs.google.com/spreadsheets/d/1HQRMJgu_zArp-sLnvFMDzOyjdsht87eFLECxMK858lA/edit?gid=0)**
   — community-maintained per-arc episode lists keyed by MKV CRC32. We use
   it for two things: (a) when a file's CRC matches a sheet entry, we
   override the filename-derived arc and episode index with the sheet's
   authoritative mapping; (b) when SpykerNZ has no rich data for an arc,
   the sheet synthesizes minimal `Episode` records from its
   chapter/episode/release-date columns.

**Coverage caveat for the sheet.** The sheet lists CRCs for the **latest
re-encode** of each episode; older releases in your library will not match
by CRC and will fall back to the filename-derived arc + episode-number path
(which is fine — that's the original behavior). If your library is mostly
recent encodes, CRC override will fire often; if mostly older, it will
mostly be a no-op. Either way the data still flows from SpykerNZ + sheet
synthesis, so coverage doesn't degrade.

**Arc-name aliases** (e.g. "Whiskey Peak" ↔ "Whisky Peak", "Arabasta" ↔
"Alabasta", "Wano Act 1" ↔ "Wano") live in two small maps:
`src/source/spykernz.rs::arc_alias` and `src/source/sheet.rs::arc_alias`.
Add new entries as the community renames things.

## Install

Three options: pre-built binary (fastest), `cargo install`, or build from
source. Docker + Docker Compose v2 are only needed for the optional Jellyfin
integration harness in `docker/`.

<!-- install-snippet:start -->
<!--
The block between these markers is kept byte-identical to .github/install-snippet.md
(the workflow uses that file to populate every release body). If you edit one,
edit the other. `VERSION` below is what the workflow substitutes for `${TAG}`.
-->

### Pre-built binaries

Pre-built binaries for Linux, macOS, and Windows are attached to every
[GitHub release](https://github.com/TrimVis/PaceFinder/releases). The
snippets below install into `~/.local/bin` (no `sudo` needed); make sure
that directory is on your `PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"   # add to your shell rc if not already there
mkdir -p "$HOME/.local/bin"
```

Replace `VERSION` with the tag you want (e.g. `v0.2.0`), or use
`latest/download` to always grab the newest release.

**Linux (x86_64, musl static):**

```sh
VERSION=v0.2.0
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-x86_64-unknown-linux-musl.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
pacefinder version
```

**Linux (aarch64, musl static):**

```sh
VERSION=v0.2.0
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-aarch64-unknown-linux-musl.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
pacefinder version
```

**macOS (Apple Silicon):**

```sh
VERSION=v0.2.0
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-aarch64-apple-darwin.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
pacefinder version
```

**macOS (Intel):**

```sh
VERSION=v0.2.0
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-x86_64-apple-darwin.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
pacefinder version
```

**Windows (x86_64, PowerShell):**

```powershell
$Version = "v0.2.0"
$Dest = "$HOME\bin"
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
Invoke-WebRequest -Uri "https://github.com/TrimVis/PaceFinder/releases/download/$Version/pacefinder-x86_64-pc-windows-msvc.zip" -OutFile "$env:TEMP\pacefinder.zip"
Expand-Archive -Force "$env:TEMP\pacefinder.zip" -DestinationPath $Dest
# Add $Dest to your PATH if it isn't already
& "$Dest\pacefinder.exe" version
```

**Auto-detect OS and architecture (Linux/macOS):**

```sh
VERSION=v0.2.0
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)   TARGET=x86_64-unknown-linux-musl ;;
  Linux-aarch64)  TARGET=aarch64-unknown-linux-musl ;;
  Darwin-arm64)   TARGET=aarch64-apple-darwin ;;
  Darwin-x86_64)  TARGET=x86_64-apple-darwin ;;
  *) echo "unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
mkdir -p "$HOME/.local/bin"
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-${TARGET}.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
pacefinder version
```

### Verifying checksums

Each archive ships with a sibling `.sha256` file. Verify before installing:

```sh
VERSION=v0.2.0
ARCHIVE=pacefinder-x86_64-unknown-linux-musl.tar.gz
curl -fsSLO "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/${ARCHIVE}"
curl -fsSLO "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/${ARCHIVE}.sha256"
sha256sum -c "${ARCHIVE}.sha256"
```
<!-- install-snippet:end -->

### From crates.io / source

```sh
# from a cloned checkout
cargo install --path .

# requires: Rust stable 1.85+ (for edition 2024).
# rustup will pick this up from rust-toolchain.toml automatically.
```

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

# tidy orphan arc folders (rmdir if empty, .ignore if foreign content)
pacefinder cleanup "/path/to/library/One Pace" --dry-run
pacefinder cleanup "/path/to/library/One Pace"
pacefinder cleanup "/path/to/library/One Pace" --remove  # undo: delete our .ignore files

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
| `cleanup <series-root>` | Tidy orphan arc folders: `rmdir` if empty, write `.ignore` if they hold only foreign content. `--remove` undoes our `.ignore` writes. |
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

### When to run `cleanup`

Reach for `pacefinder cleanup <series-root>` after a successful `generate`
run when either:

- a previous tagging tool (Sonarr, tinyMediaManager, etc.) left foreign
  `*.nfo` files inside arc folders that pacefinder didn't match — those
  folders still show up in Jellyfin as ghost seasons. `cleanup` drops a
  `.ignore` so Jellyfin skips them without you having to delete anyone's
  data; or
- pacefinder has matched everything it can, but a few empty arc folders
  remain (stale rename targets, scratch dirs). `cleanup` `rmdir`s them.

`cleanup` never touches folders that contain at least one pacefinder-written
`.nfo`, never deletes media files, and is reversible for the `.ignore` case
via `pacefinder cleanup <series-root> --remove`. After applying, trigger a
Jellyfin library scan; if ghost seasons persist, remove + re-add the library
in Jellyfin's admin UI (this is a Jellyfin internals issue, not something
the CLI can fix from outside).

## Troubleshooting Jellyfin

**My re-run didn't update the metadata in Jellyfin.** A library scan in
Jellyfin discovers new/moved files but does **not** re-parse NFOs for items
already in its database. When pacefinder rewrites an NFO (because of a new
release or upstream change), Jellyfin won't pick up the changes on a plain
library scan. Force it with one of:

- **In the UI:** right-click the *One Pace* series → *Refresh metadata* →
  pick *Replace all metadata* + tick *Replace existing images*.
- **Via API:**
  ```sh
  curl -X POST -H "Authorization: MediaBrowser Token=$TOKEN" \
    "http://<host>:8096/Items/$SERIES_ID/Refresh?metadataRefreshMode=FullRefresh&replaceAllMetadata=true&imageRefreshMode=FullRefresh&replaceAllImages=true&recursive=true"
  ```
  `$SERIES_ID` comes from `GET /Items?IncludeItemTypes=Series&recursive=true`.

**Ghost seasons with weird numbers like `Season 155217`.** Earlier scans
registered arc folders as seasons before pacefinder could write `season.nfo`.
Run `pacefinder cleanup <series-root>` to remove or `.ignore` the orphans,
then trigger a library scan. If the ghosts persist, the Jellyfin DB is
holding cached entries — remove and re-add the library in Dashboard →
Libraries.

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
testlib/          (gitignored) sample One Pace media for local dev
```

## License

MIT — see [LICENSE](LICENSE).
