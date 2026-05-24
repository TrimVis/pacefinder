# PaceFinder

**A CLI for managing a [One Pace](https://onepace.net) fan-cut media
library. It writes Kodi-format NFO sidecars + posters so Jellyfin / Plex /
Kodi show each arc as a proper season, queues missing releases to
qBittorrent, and keeps the on-disk layout tidy.**

Three things, one parser:

- **`generate`** — match each file to its arc/episode against a chain of
  upstream sources (onepace.net, SpykerNZ NFO bundle, community CRC
  sheet — see [docs/data-sources.md](docs/data-sources.md)) and write
  NFOs + posters next to your media.
- **`download`** — diff your library against upstream `/releases` and
  queue what's missing to qBittorrent, each torrent's `save_path` aimed
  at the right arc folder. See [docs/download.md](docs/download.md).
- **`reorder`** / **`cleanup`** — one-time wrap of flat arc folders into
  a series wrapper; ongoing tidy of orphan folders that Jellyfin would
  otherwise surface as ghost seasons.

## Install

<!-- install-snippet:start -->
<!--
The block between these markers is kept byte-identical to .github/install-snippet.md
(the workflow uses that file to populate every release body). If you edit one,
edit the other. `VERSION` below is what the workflow substitutes for `${TAG}`.
-->

### Pre-built binaries

Pre-built binaries for Linux, macOS, and Windows are attached to every
[GitHub release](https://github.com/TrimVis/PaceFinder/releases). Default
install destination is `~/.local/bin` (no `sudo` needed); make sure it's
on your `PATH`.

**Linux / macOS — auto-detect OS and architecture:**

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

<details>
<summary>Per-platform commands (Linux x86_64 / aarch64, macOS Intel / Apple Silicon, Windows)</summary>

**Linux (x86_64, musl static):**

```sh
VERSION=v0.2.0
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-x86_64-unknown-linux-musl.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
```

**Linux (aarch64, musl static):**

```sh
VERSION=v0.2.0
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-aarch64-unknown-linux-musl.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
```

**macOS (Apple Silicon):**

```sh
VERSION=v0.2.0
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-aarch64-apple-darwin.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
```

**macOS (Intel):**

```sh
VERSION=v0.2.0
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-x86_64-apple-darwin.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
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

</details>

<details>
<summary>Verifying checksums</summary>

Each archive ships with a sibling `.sha256` file. Verify before installing:

```sh
VERSION=v0.2.0
ARCHIVE=pacefinder-x86_64-unknown-linux-musl.tar.gz
curl -fsSLO "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/${ARCHIVE}"
curl -fsSLO "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/${ARCHIVE}.sha256"
sha256sum -c "${ARCHIVE}.sha256"
```

</details>
<!-- install-snippet:end -->

### From crates.io / source

```sh
cargo install --path .   # from a cloned checkout
```

Requires Rust stable 1.85+ (for edition 2024). `rustup` picks the right
channel up from `rust-toolchain.toml` automatically.

## Quick start

```sh
# (if your library is flat) wrap arc folders in a series wrapper
pacefinder reorder /path/to/library

# write NFOs + posters so Jellyfin shows arcs as seasons
pacefinder generate "/path/to/library/One Pace"

# (optional) queue everything you're missing to qBittorrent
PACEFINDER_QBT_PASS=hunter2 pacefinder download "/path/to/library/One Pace"
```

| Command | Description |
|---|---|
| `generate <series-root>` | Write NFOs + posters next to every recognized file. |
| `download <series-root>` | Queue missing releases to qBittorrent. Per-arc `save_path`, optional `--prepopulate-nfo`. See [docs/download.md](docs/download.md). |
| `scan <path>` | List recognized video files. Useful diagnostic. |
| `reorder <path>` | One-time setup: wrap top-level arc folders inside a series folder when your layout is flat. |
| `cleanup <series-root>` | `rmdir` empty arc folders, write `.ignore` into folders with only foreign content. `--remove` undoes our `.ignore` writes. |
| `cache path` / `cache clear` | Show where cached upstream responses live, or wipe them. |
| `version` | Print version. |

Global flags: `-v` / `-vv` (more verbose), `-q` / `-qq` / `-qqq` (less),
`--log <directive>` (power-user `tracing-subscriber` filter; also reads
`PACEFINDER_LOG`).

Notable `generate` flags (run `--help` for the full set): `--dry-run`,
`--refresh` (bypass cache), `--cache-ttl 7d`, `--force` /
`--non-interactive` (overwrite-conflict policy), `--display-order`
(`absolute` default — flat 1..N episode list across arcs; `aired` for the
season-card grouping), `--lock` (`show` default — emit
`<lockdata>true</lockdata>` so Jellyfin's metadata explorer stops
rewriting NFOs; `all` to lock season/episode NFOs too).

## Required library layout

Jellyfin (and Plex, Kodi) treat every immediate child of a TV library as
a separate Series. The path you pass to `pacefinder generate` must be the
series folder, with arc folders directly inside:

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
    [One Pace][8-21] Orange Town [1080p]/
      ...
```

If your library is flat (arc folders at the top with no `One Pace/`
wrapper), run `pacefinder reorder <library>` first — `generate` warns when
it detects this layout to prevent the silent-failure case where each arc
becomes its own Jellyfin series.

In Jellyfin, add `<jellyfin-library-root>/` as a TV Shows library with the
*Nfo* local metadata reader enabled and remote metadata fetchers off.

## More

- [docs/data-sources.md](docs/data-sources.md) — where metadata comes
  from, the source chain, arc-name aliases, adding a new source.
- [docs/download.md](docs/download.md) — `download` subcommand details:
  qBittorrent setup, save-path traps, `--prepopulate-nfo`, scope.
- [docs/troubleshooting.md](docs/troubleshooting.md) — Jellyfin
  metadata-refresh trap, ghost seasons, "nothing matched" debugging.
- [docs/development.md](docs/development.md) — build, dev loop with the
  Docker Jellyfin harness, project layout, release process.

## License

MIT — see [LICENSE](LICENSE).
