# Download — queue missing releases to qBittorrent

`pacefinder download <series-root>` scrapes onepace.net's `/releases` page,
diffs the available magnets against your library and qBittorrent's
current queue, and queues anything missing — with each torrent's
`save_path` set to the right arc folder under your library.

Queue-and-go: no waiting, no progress bars, no auto-move-on-complete.
Compose with [`generate`](../README.md#run) to fill in NFOs once the
files land, or pass `--prepopulate-nfo` to get the episode metadata
written ahead of time.

## What's in scope

- **Torrent only.** Pixeldrain links exist on `/releases` but free-tier
  rate limits make them unsuitable for full-library downloads; the
  torrent flow is the canonical channel anyway.
- **qBittorrent only.** Trait-based design isn't there yet — when a
  second client (Transmission, Deluge, …) gets implemented, this CLI
  flag set grows by one `--client` choice.
- **`/releases` as the upstream listing.** Other sources (SpykerNZ,
  Google Sheet) are not consulted for download URLs; they only inform
  the pre-populate NFO step.

## Quick start

```sh
# preview what would happen — no qBittorrent connection needed
pacefinder download "/path/to/library/One Pace" --dry-run

# actually queue with default qBittorrent at localhost:8080
PACEFINDER_QBT_PASS=your-password \
  pacefinder download "/path/to/library/One Pace"

# only do one arc, prepopulate NFOs so Jellyfin has metadata immediately
pacefinder download "/path/to/library/One Pace" \
  --only-arc "Egghead" \
  --prepopulate-nfo
```

## Credentials

| | Flag | Env var | Default |
|---|---|---|---|
| Web UI URL | `--qbt-url` | `PACEFINDER_QBT_URL` | `http://localhost:8080` |
| Username | `--qbt-user` | `PACEFINDER_QBT_USER` | `admin` |
| Password | `--qbt-pass` | `PACEFINDER_QBT_PASS` | `adminadmin` |

Use the env var for the password — flags end up in shell history.

The defaults match qBittorrent's out-of-the-box install. **Change them
the first time you log into the Web UI.** If you've enabled
"Bypass authentication for clients on localhost" you can leave the
password as anything; auth still happens but qBittorrent ignores it.

## Diff logic — what counts as "missing"

For each release on `/releases`:

1. Parse the magnet's `dn=` parameter into arc, episode, resolution, CRC.
2. Skip if resolution > `--resolution` cap (default `1080p`).
3. Group per `(arc, episode)`, keep the highest-resolution variant.
4. Skip if the release's CRC matches a file already in your library
   (unless `--refresh-existing` is set).
5. Skip if the release's CRC matches a torrent already in qBittorrent's
   queue (parsed from torrent name).
6. Queue everything else.

CRC equality is the equivalence test. If you have an older encode of an
episode, the new release has a different CRC and we'll see it as missing
— pass `--refresh-existing` if you don't actually want the upgrade.

## Save-path traps

`save_path` is set per torrent to `<library>/<arc-folder>`. PaceFinder
matches existing arc folders by normalized arc name; if no match it
proposes `<library>/[One Pace] <Arc Name>`. The path qBittorrent
receives is **always from qBittorrent's POV**, not pacefinder's — so
when the two don't share a filesystem view, you need `--save-path-map`.

### `--save-path-map HOST=CONTAINER`

Translates each `save_path` from the host filesystem to qBittorrent's
view. Standard *arr-app "Remote Path Mappings" pattern.

```sh
# library at /mnt/media/anime/One Pace on the host;
# qBittorrent runs in a container with /mnt/media bind-mounted to /downloads
pacefinder download "/mnt/media/anime/One Pace" \
    --save-path-map "/mnt/media=/downloads"
```

Internally: `<host>/<rest>` → `<container>/<rest>`. PaceFinder verifies
the host prefix is actually a parent of the library root and errors
early if not (typo guard). Also reads from `PACEFINDER_SAVE_PATH_MAP`,
e.g. `PACEFINDER_SAVE_PATH_MAP=/mnt/media=/downloads`.

Dry-run shows both the translated `save_path` and the original
`host_path` so you can sanity-check before live-queueing.

### When you'll need it

- **qBittorrent runs in a Docker container** with a different mount
  table than the host.
- **qBittorrent runs as a different user** with a different chroot or
  bind-mount setup (less common, same fix).

### When you won't

- **Native qBittorrent on the same machine** as where you run pacefinder
  — the path is the same on both sides; no mapping needed.

### Heuristic warning

On a non-dry-run, after logging in, pacefinder asks qBittorrent for its
`defaultSavePath` (`GET /api/v2/app/defaultSavePath`). If that path and
your library root share no prefix and you didn't pass
`--save-path-map`, you get a `WARN` line suggesting a likely mapping.
It's a hint, not a block — false positives are possible (your library
might be elsewhere on purpose).

### Permission caveat

`--save-path-map` only handles the path translation. The qBittorrent
user still needs write permission to the destination — that's a Docker /
filesystem concern outside pacefinder's reach.

## `--prepopulate-nfo`

When set, after queueing each torrent we also write an `episode.nfo` at
`<save_path>/<basename>.nfo` (with `<lockdata>true</lockdata>` so
Jellyfin doesn't try to overwrite it once the .mkv arrives). The
metadata comes from the same composite source chain `generate` uses:
SpykerNZ for rich title+plot when it has the arc, falling through to
the Google Sheet's synthesized records otherwise.

Out of scope for `--prepopulate-nfo`:

- `tvshow.nfo` (series-level) — run `pacefinder generate` once for the
  initial setup.
- `season.nfo` and arc poster — same. The episode-level NFO is what
  benefits most from being there before the file arrives.
- Overwriting existing NFOs — if the target path already has an `.nfo`,
  we skip it. Run `pacefinder generate --force` later if you want a
  rewrite.

## Known limitations

- **`/releases` schema brittleness.** Same risk as the onepace.net `/watch`
  adapter: a site rebuild could reshuffle the RSC payload. If `download`
  starts returning zero releases, that's the first thing to suspect.
- **Magnet `dn=` is not part of the BitTorrent spec.** If a release ever
  shows up without `dn=`, we can't identify it pre-fetch and skip it
  with a warning. Hasn't happened in any sampled data so far.
- **Resolution detection is filename-based.** A release tagged `[1080p]`
  in its filename but actually encoded at a lower resolution would
  still be picked. We don't probe the file.
- **CRC drift on re-uploads.** When upstream re-encodes the same
  episode, the new CRC won't match your library and pacefinder will
  treat it as missing — you'll re-download it. `--refresh-existing` is
  the explicit knob; default behavior trusts CRC equality.
- **No `.torrent` metadata fetch.** We don't fetch the torrent's piece
  list from DHT, so we can't verify total file size before queueing.
  Whatever qBittorrent's download-limit / disk-space behavior is, it
  applies as-is.

## See also

- [Data sources](data-sources.md) — where metadata comes from for the
  prepopulate step.
- [Troubleshooting](troubleshooting.md) — Jellyfin metadata-refresh
  trap (relevant after downloads complete).
