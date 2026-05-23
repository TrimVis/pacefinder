# Troubleshooting

## My re-run didn't update the metadata in Jellyfin

A library scan in Jellyfin discovers new/moved files but does **not**
re-parse NFOs for items already in its database. When pacefinder rewrites
an NFO (because of a new release or upstream change), Jellyfin won't pick
up the changes on a plain library scan.

Force it with one of:

- **In the UI:** right-click the *One Pace* series → *Refresh metadata* →
  pick *Replace all metadata* + tick *Replace existing images*.
- **Via the API** (replace `$TOKEN` and `$SERIES_ID`):
  ```sh
  curl -X POST -H "Authorization: MediaBrowser Token=$TOKEN" \
    "http://<host>:8096/Items/$SERIES_ID/Refresh?metadataRefreshMode=FullRefresh&replaceAllMetadata=true&imageRefreshMode=FullRefresh&replaceAllImages=true&recursive=true"
  ```
  `$SERIES_ID` comes from
  `GET /Items?IncludeItemTypes=Series&recursive=true`.

## Ghost seasons with weird numbers like "Season 155217"

Earlier scans registered arc folders as seasons before pacefinder could
write a `season.nfo` for them. Jellyfin parsed the chapter range from the
folder name (e.g. `[155-217]`) and turned it into a phantom season number.

Two-step fix:

1. Run `pacefinder cleanup <series-root>` — `rmdir`s empty arc folders and
   writes `.ignore` into folders that only contain foreign metadata
   (NFOs/thumbs from another tagger).
2. Trigger a Jellyfin library scan. If the ghosts persist, the Jellyfin DB
   still has cached entries — remove and re-add the library in Dashboard →
   Libraries. This is a Jellyfin internals issue; the CLI can't fix it from
   outside.

## "Nothing matched" / `no One Pace files matched`

`pacefinder generate` walks the path you gave it and looks for video files
whose names follow the One Pace release scheme. If nothing matches, it
tells you the total video count and an example expected filename.

Common causes:

- **Wrong path level.** Passed the library root instead of the series
  folder. PaceFinder detects this case and warns; re-run on the series
  folder one level deeper (e.g. `<library>/One Pace/`).
- **Custom/old release naming.** If your files don't have the
  `[One Pace][<range>] <Arc> <ep> [<res>][<CRC>].mkv` pattern, the parser
  doesn't recognize them. Rename them or open an issue with a sample.

## Permission / write errors during `generate`

PaceFinder writes NFO and PNG files next to your media. Filesystem
permissions need to allow writes to those directories. If you mount media
read-only (e.g. via NFS), consider running PaceFinder on the source-of-
truth host that has write access, then letting the read-only mount expose
the generated NFOs.
