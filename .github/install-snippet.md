<!--
Canonical install snippet for PaceFinder pre-built binaries.

Single source of truth for:
  - The "Pre-built binaries" subsection of README.md (## Install)
  - The auto-generated release body produced by .github/workflows/release.yml

If you edit one, edit the other (keep them byte-identical between the
`<!-- install-snippet:start --` and `<!-- install-snippet:end --` markers
in README.md). The release workflow uploads this file verbatim as the
release body using `gh release edit --notes-file`.

`${TAG}` and `${VERSION}` are substituted by the workflow at release time.
In README.md the snippet uses a literal `vX.Y.Z` so users can swap in any
version (or use the `/releases/latest/download/` redirect).
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
VERSION=${TAG}
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-x86_64-unknown-linux-musl.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
pacefinder version
```

**Linux (aarch64, musl static):**

```sh
VERSION=${TAG}
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-aarch64-unknown-linux-musl.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
pacefinder version
```

**macOS (Apple Silicon):**

```sh
VERSION=${TAG}
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-aarch64-apple-darwin.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
pacefinder version
```

**macOS (Intel):**

```sh
VERSION=${TAG}
curl -fsSL "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/pacefinder-x86_64-apple-darwin.tar.gz" \
  | tar -xz -C "$HOME/.local/bin" pacefinder
chmod +x "$HOME/.local/bin/pacefinder"
pacefinder version
```

**Windows (x86_64, PowerShell):**

```powershell
$Version = "${TAG}"
$Dest = "$HOME\bin"
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
Invoke-WebRequest -Uri "https://github.com/TrimVis/PaceFinder/releases/download/$Version/pacefinder-x86_64-pc-windows-msvc.zip" -OutFile "$env:TEMP\pacefinder.zip"
Expand-Archive -Force "$env:TEMP\pacefinder.zip" -DestinationPath $Dest
# Add $Dest to your PATH if it isn't already
& "$Dest\pacefinder.exe" version
```

**Auto-detect OS and architecture (Linux/macOS):**

```sh
VERSION=${TAG}
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
VERSION=${TAG}
ARCHIVE=pacefinder-x86_64-unknown-linux-musl.tar.gz
curl -fsSLO "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/${ARCHIVE}"
curl -fsSLO "https://github.com/TrimVis/PaceFinder/releases/download/${VERSION}/${ARCHIVE}.sha256"
sha256sum -c "${ARCHIVE}.sha256"
```
