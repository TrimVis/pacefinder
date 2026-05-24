# Download — queue missing releases to qBittorrent

`pacefinder download <series-root>` scrapes onepace.net's `/releases` page,
diffs the available magnets against your library and qBittorrent's
current queue, and queues anything missing — with each torrent's
`save_path` set to the right arc folder under your library.

Queue-and-go: no waiting, no progress bars. Pair with `generate` to fill
in NFOs once the files land, or pass `--prepopulate-nfo`.

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

## Diff logic — what counts as "missing"

For each release on `/releases`:

1. Parse the magnet's `dn=` parameter into arc, episode, resolution, CRC.
2. Skip if resolution > `--resolution` cap (default `1080p`).
3. Group per `(arc, episode)`, keep the highest-resolution variant.
4. Skip if the release's CRC matches a file already in your library
   (unless `--requeue-existing` is set).
5. Skip if the release's CRC matches a torrent already in qBittorrent's
   queue (parsed from torrent name).
6. Queue everything else.

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

### Heuristic warning

If qBittorrent's `defaultSavePath` and your library root share no prefix
and you didn't pass `--save-path-map`, you get a `WARN` suggesting a
mapping. Hint only — false positives are possible.

## `--prepopulate-nfo`

After queueing each torrent we write an `episode.nfo` at
`<save_path>/<basename>.nfo` (with `<lockdata>true</lockdata>` so
Jellyfin doesn't overwrite it once the .mkv arrives). Series/season NFOs
and existing-NFO overwrites are `generate`'s job.

## Extended cuts (`--prefer-extended`)

Some episodes ship as both a regular release and an Extended cut. By
default, Extended releases are skipped — they show up as separate
torrents with a different filename pattern (`<Arc> <N> Extended`).

`--prefer-extended` (or `PACEFINDER_PREFER_EXTENDED=1`) queues the
Extended variant instead of the regular for any `(arc, episode)` that
has both upstream. Preference wins over the resolution cap: a 720p
Extended beats a 1080p regular under this flag. If only one variant
exists upstream for an episode, you get that one regardless.

The "have" check stays CRC-based — if you've already got the regular
on disk, queueing the Extended will *add* it (you'll end up with both
files for that episode until you tidy up). Run `pacefinder cleanup
--remove-superseded` afterwards to move the regulars out of the way
(see [troubleshooting](troubleshooting.md) once that lands).

## Known limitations

- **`/releases` schema brittleness.** A site rebuild could reshuffle the
  RSC payload. If `download` starts returning zero releases, that's the
  first thing to suspect.
- **Magnet `dn=` is not part of the BitTorrent spec.** A release without
  `dn=` can't be identified pre-fetch and is skipped with a warning.
- **Resolution detection is filename-based.** We don't probe the file —
  a `[1080p]`-tagged release that's actually lower-res still gets picked.
- **CRC drift on re-uploads.** A re-encoded episode has a new CRC; we
  treat it as missing. `--requeue-existing` is the explicit knob if you
  don't want the upgrade.

## See also

- [Data sources](data-sources.md) — where metadata comes from for the
  prepopulate step.
- [Troubleshooting](troubleshooting.md) — Jellyfin metadata-refresh
  trap (relevant after downloads complete).
