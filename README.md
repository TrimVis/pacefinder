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

Pre-built binaries for Linux, macOS, and Windows are attached to every
[GitHub release](https://github.com/TrimVis/PaceFinder/releases/latest) —
each release body has copy-paste install commands for your platform plus
checksum verification.

From source (requires Rust stable 1.85+; `rustup` picks the right channel
from `rust-toolchain.toml`):

```sh
cargo install --path .   # from a cloned checkout
```

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
| `cleanup <series-root>` | `rmdir` empty arc folders, write `.ignore` into folders with only foreign content. `--remove` undoes our `.ignore` writes; `--migrate-extended-folders` and `--remove-superseded` handle Extended-cut layout. |
| `cache path` / `cache clear` | Show where cached upstream responses live, or wipe them. |
| `version` | Print version. |

Global flags: `-v` / `-vv` (more verbose), `-q` / `-qq` / `-qqq` (less),
`--log <directive>` (power-user `tracing-subscriber` filter; also reads
`PACEFINDER_LOG`).

Notable `generate` flags (run `--help` for the full set): `--dry-run`,
`--refresh` (bypass cache), `--cache-ttl 7d`, `--force` /
`--non-interactive` (overwrite-conflict policy), `--display-order`
(`absolute` default — flat 1..N episode list across arcs; `aired` for the
season-card grouping), `--lock` (`none` default — opt
into emitting `<lockdata>true</lockdata>` once you're happy with the
metadata; `show` locks tvshow.nfo, `all` also locks season/episode NFOs.
Heads-up: Jellyfin copies the lock state into its DB and stops
re-reading NFOs for locked items — see [docs/troubleshooting.md](docs/troubleshooting.md)).

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
