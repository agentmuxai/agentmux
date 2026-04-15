#!/usr/bin/env bash
# Package AgentMux as a portable directory + ZIP (Windows x64).
# Usage: bash scripts/package-portable.sh [output-dir]
#
# Default output: ~/Desktop/agentmux-{version}-x64-portable/

set -euo pipefail

# Resolve the repo root BEFORE any `cd` so later path lookups (e.g. the
# VERSION_HISTORY.md size-table append below) don't get confused by the
# script's internal working-directory change.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

VERSION=$(node -p "require('./package.json').version")
OUTDIR="${1:-$HOME/Desktop}"
PORTABLE="$OUTDIR/agentmux-$VERSION-x64-portable"
ZIPPATH="$OUTDIR/agentmux-$VERSION-x64-portable.zip"

echo "Packaging AgentMux v$VERSION Portable..."

# Verify required files
for f in target/release/agentmux-cef.exe dist/cef/libcef.dll dist/bin/agentmux-srv-$VERSION-windows.x64.exe dist/frontend/index.html target/release/agentmux-launcher.exe; do
    if [ ! -f "$f" ]; then
        echo "ERROR: $f not found — build first" >&2
        exit 1
    fi
done

# Clean previous
rm -rf "$PORTABLE" "$ZIPPATH"

# Create structure
mkdir -p "$PORTABLE/runtime/locales" "$PORTABLE/runtime/frontend" "$PORTABLE/runtime/tools/bin"
mkdir -p "$PORTABLE/data"

# Launcher in root
cp target/release/agentmux-launcher.exe "$PORTABLE/agentmux.exe"

# README
cat > "$PORTABLE/README.txt" <<READMEEOF
AgentMux v$VERSION - Portable Edition

Quick Start:
  1. Extract this folder (or ZIP) anywhere
  2. Run agentmux.exe

Requirements:
  - Windows 10/11 x64
  - No installation needed
  - No admin rights required

Data:
  All user data (sessions, settings, logs) is stored in the data/ folder
  next to agentmux.exe. Back it up or move it along with this folder.
READMEEOF

# data/ placeholder so the folder is visible immediately after extraction
cat > "$PORTABLE/data/README.txt" <<DATAEOF
AgentMux user data

This folder contains your sessions, settings, and logs.
It is safe to back up. Do not delete it while AgentMux is running.

  data/config/   — settings.json, keybindings.json
  data/db/       — session history and block state
  data/logs/     — host and sidecar log files
  data/cef/      — browser cache (safe to delete when app is closed)
DATAEOF

# Runtime binaries — versioned filenames so WER dumps & Event Viewer show versions
cp target/release/agentmux-cef.exe "$PORTABLE/runtime/agentmux-$VERSION.exe"
cp dist/bin/agentmux-srv-$VERSION-windows.x64.exe "$PORTABLE/runtime/"

# wsh has been retired — see specs/SPEC_RETIRE_WSH_2026_04_12.md. No binary
# to ship anymore; AGENTMUX env var is now a plain "1" sentinel.

# Frontend
cp -r dist/frontend/* "$PORTABLE/runtime/frontend/"

# CEF core
cp dist/cef/libcef.dll "$PORTABLE/runtime/"
cp dist/cef/chrome_elf.dll "$PORTABLE/runtime/" 2>/dev/null || true
cp dist/cef/icudtl.dat "$PORTABLE/runtime/" 2>/dev/null || true
cp dist/cef/v8_context_snapshot.bin "$PORTABLE/runtime/" 2>/dev/null || true

# GPU support
cp dist/cef/libEGL.dll dist/cef/libGLESv2.dll dist/cef/d3dcompiler_47.dll "$PORTABLE/runtime/" 2>/dev/null || true

# Resource paks
cp dist/cef/chrome_100_percent.pak dist/cef/chrome_200_percent.pak dist/cef/resources.pak "$PORTABLE/runtime/" 2>/dev/null || true

# Locale (en-US only)
cp dist/cef/locales/en-US.pak "$PORTABLE/runtime/locales/" 2>/dev/null || true

# Bundled tools — jq and rg ship inside runtime/tools/bin/ so agents work
# offline without any /tools install step.
TOOLS_BIN="$PORTABLE/runtime/tools/bin"
TOOLS_CACHE="$HOME/.agentmux/tool-build-cache"
mkdir -p "$TOOLS_CACHE"

bundle_tool() {
    local name="$1" url="$2" sha256_expected="$3" archive_path="$4"
    local dest="$TOOLS_BIN/$name"
    local cache_key="$TOOLS_CACHE/${name}-$(echo "$url" | md5sum | cut -c1-8)"

    # Use cached download if present and sha256 matches
    if [ -f "$cache_key" ]; then
        local actual
        actual=$(sha256sum "$cache_key" | cut -d' ' -f1)
        if [ "$actual" = "$sha256_expected" ]; then
            echo "  [tools] $name: using cached download"
        else
            echo "  [tools] $name: cache sha256 mismatch, re-downloading"
            rm -f "$cache_key"
        fi
    fi

    if [ ! -f "$cache_key" ]; then
        echo "  [tools] $name: downloading from $url"
        curl -fsSL "$url" -o "$cache_key"
        local actual
        actual=$(sha256sum "$cache_key" | cut -d' ' -f1)
        if [ "$actual" != "$sha256_expected" ]; then
            echo "ERROR: sha256 mismatch for $name! expected=$sha256_expected got=$actual" >&2
            rm -f "$cache_key"
            exit 1
        fi
    fi

    if [ -z "$archive_path" ]; then
        # Direct binary
        cp "$cache_key" "$dest"
    else
        # ZIP: extract specific file
        local tmpdir
        tmpdir=$(mktemp -d)
        unzip -q "$cache_key" "$archive_path" -d "$tmpdir"
        cp "$tmpdir/$archive_path" "$dest"
        rm -rf "$tmpdir"
    fi
    chmod +x "$dest" 2>/dev/null || true
    echo "  [tools] $name: bundled → $dest"
}

echo "Bundling tools into runtime/tools/bin/ ..."
bundle_tool \
    "jq.exe" \
    "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-windows-amd64.exe" \
    "7451fbbf37feffb9bf262bd97c54f0da558c63f0748e64152dd87b0a07b6d6ab" \
    ""
bundle_tool \
    "rg.exe" \
    "https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/ripgrep-14.1.1-x86_64-pc-windows-msvc.zip" \
    "d0f534024c42afd6cb4d38907c25cd2b249b79bbe6cc1dbee8e3e37c2b6e25a1" \
    "ripgrep-14.1.1-x86_64-pc-windows-msvc/rg.exe"

# Verify versions match
CEF_VER=$(grep -ao "$VERSION" "$PORTABLE/runtime/agentmux-$VERSION.exe" | head -1)
SRV_VER=$(grep -ao "$VERSION" "$PORTABLE/runtime/agentmux-srv-$VERSION-windows.x64.exe" | head -1)
if [ "$CEF_VER" != "$VERSION" ] || [ "$SRV_VER" != "$VERSION" ]; then
    echo "ERROR: Binary version mismatch! CEF=$CEF_VER SRV=$SRV_VER expected=$VERSION" >&2
    exit 1
fi

# Size
DIR_SIZE=$(du -sh "$PORTABLE" | cut -f1)

# ZIP
#
# Tries pwsh 7 first (Compress-Archive with proper module), then falls back to
# Windows PowerShell 5, then to `tar -a -cf` (bsdtar on Win10+). If *none*
# succeed the script exits non-zero instead of silently reporting "N/A", which
# is how a broken packaging run used to produce a portable folder on the
# desktop with no ZIP beside it.
cd "$OUTDIR"
ZIP_NAME="agentmux-$VERSION-x64-portable.zip"
rm -f "$ZIP_NAME"

portable_basename=$(basename "$PORTABLE")
zip_made=0

if command -v pwsh >/dev/null 2>&1; then
    if pwsh -Command "Compress-Archive -Path '${portable_basename}/*' -DestinationPath '$ZIP_NAME' -Force" >/dev/null 2>&1; then
        zip_made=1
    fi
fi

if [ "$zip_made" -eq 0 ]; then
    if powershell -Command "Compress-Archive -Path '${portable_basename}/*' -DestinationPath '$ZIP_NAME' -Force" >/dev/null 2>&1; then
        zip_made=1
    fi
fi

if [ "$zip_made" -eq 0 ]; then
    # `-C "$portable_basename" .` archives the contents at the ZIP root,
    # matching Compress-Archive's `-Path '…/*'` behavior above. Without
    # `-C`, bsdtar wraps everything under a `$portable_basename/` directory
    # which breaks consumers expecting a flat layout (launcher + runtime/
    # at the ZIP root).
    if tar -a -cf "$ZIP_NAME" -C "$portable_basename" . >/dev/null 2>&1; then
        zip_made=1
    fi
fi

if [ "$zip_made" -eq 0 ] || [ ! -f "$ZIP_NAME" ]; then
    echo "ERROR: failed to create $ZIP_NAME — Compress-Archive (pwsh + powershell) and tar -a both failed" >&2
    exit 1
fi

ZIP_SIZE=$(du -sh "$ZIP_NAME" 2>/dev/null | cut -f1 || echo "N/A")

# Append compact size row to VERSION_HISTORY.md under the "## Sizes" table
# if it exists. REPO_ROOT was captured at the top of the script BEFORE the
# `cd "$OUTDIR"` above, so it still points at the repo root even though cwd
# is now the output directory. Skipped silently if the file / section
# isn't there.
if [ -f "$REPO_ROOT/VERSION_HISTORY.md" ] && grep -q "^## Sizes " "$REPO_ROOT/VERSION_HISTORY.md" 2>/dev/null; then
    DIR_BYTES=$(find "$PORTABLE" -type f -printf '%s\n' 2>/dev/null | awk '{s+=$1} END {print s+0}')
    ZIP_BYTES=$(stat -c '%s' "$ZIP_NAME" 2>/dev/null || echo 0)
    DIR_MIB=$(awk "BEGIN {printf \"%.1f\", $DIR_BYTES/1024/1024}")
    ZIP_MIB=$(awk "BEGIN {printf \"%.1f\", $ZIP_BYTES/1024/1024}")
    TODAY=$(date +%Y-%m-%d)
    ROW="| $VERSION | $TODAY | $ZIP_MIB MiB | $DIR_MIB MiB | |"

    # Insert the row immediately after the header separator line of the
    # Sizes table (the `|---|---|...` line). awk in-place via temp file.
    awk -v row="$ROW" '
        /^\| *Version *\| *Date *\| *ZIP/ { in_sizes=1 }
        { print }
        in_sizes && /^\|[-:| ]+\|[-:| ]+\|[-:| ]+\|[-:| ]+\|[-:| ]+\|$/ {
            print row
            in_sizes=0
        }
    ' "$REPO_ROOT/VERSION_HISTORY.md" > "$REPO_ROOT/VERSION_HISTORY.md.tmp" \
        && mv "$REPO_ROOT/VERSION_HISTORY.md.tmp" "$REPO_ROOT/VERSION_HISTORY.md"
fi

echo ""
echo "[SUCCESS] Portable v$VERSION"
echo "  Directory: $PORTABLE ($DIR_SIZE)"
echo "  ZIP: $ZIPPATH ($ZIP_SIZE)"
