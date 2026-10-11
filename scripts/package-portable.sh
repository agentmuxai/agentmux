#!/usr/bin/env bash
# Package AgentMux as a portable directory + .7z archive (Windows x64).
# Usage: bash scripts/package-portable.sh [output-dir]
#
# Default output: ~/Desktop/agentmux-{version}-x64-portable/

set -euo pipefail

# Resolve the repo root BEFORE any `cd` so later path lookups (e.g. the
# VERSION_HISTORY.md size-table append below) don't get confused by the
# script's internal working-directory change.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# VERSION is the semver core (from package.json) — drives binary
# filenames, the in-binary version, and the verification grep below.
# LABEL is the ephemeral, traceable build label set by scripts/package.sh
# (`<version>+g<sha>[.dirty].<stamp>`); it names the on-disk artifacts so
# every local build is unique and tellable-apart. Falls back to VERSION
# when this script is run directly (e.g. a release build that didn't go
# through package.sh).
VERSION=$(node -p "require('./package.json').version")
LABEL="${AGENTMUX_BUILD_LABEL:-$VERSION}"
# CHANNEL matches the AGENTMUX_BUILD_CHANNEL_DEFAULT compiled into the binaries
# (scripts/package.sh exports it; release CI uses "stable", task package uses a
# local-… channel). Same fallback as package-macos.sh so the README data path
# matches DataPaths::resolve exactly (~/.agentmux/channels/<channel>/…).
CHANNEL="${AGENTMUX_BUILD_CHANNEL_DEFAULT:-stable}"
OUTDIR="${1:-$HOME/Desktop}"
PORTABLE="$OUTDIR/agentmux-$LABEL-x64-portable"
ARCHIVE_PATH="$OUTDIR/agentmux-$LABEL-x64-portable.7z"

echo "Packaging AgentMux $LABEL Portable..."

# Verify required files
for f in target/release/agentmux-cef.exe dist/cef/libcef.dll dist/bin/agentmux-srv-$VERSION-windows.x64.exe dist/frontend/index.html target/release/agentmux-launcher.exe target/release/agentmux-bashwrap.exe target/release/agentmux-mcp.exe; do
    if [ ! -f "$f" ]; then
        echo "ERROR: $f not found — build first" >&2
        exit 1
    fi
done

# Phase 3 sandbox preflight (issue #1374). A sandbox build (the default
# `--features sandbox`) produces CEF's `bootstrap.exe` plus the cdylib
# `agentmux_cef.dll`; the host entry point is bootstrap (renamed), which loads
# the DLL and passes a real `sandbox_info` to `CefExecuteProcess`.
# NOTE: the cdylib is emitted unconditionally (crate-type = ["cdylib","rlib"]),
# so its presence does NOT indicate a sandbox build. `bootstrap.exe` is only
# produced by cef-dll-sys when the `cef/sandbox` feature is active — use it
# as the sandbox discriminator throughout this script.
# If bootstrap.exe is present but the DLL is missing, packaging is broken.
if [ -f target/release/bootstrap.exe ] && [ ! -f target/release/agentmux_cef.dll ]; then
    echo "ERROR: bootstrap.exe (sandbox build) found but target/release/agentmux_cef.dll is missing —" >&2
    echo "       cannot stage the Phase-3 host. Re-run 'task build:host' (sandbox feature) and retry." >&2
    exit 1
fi

# Refuse to wipe a portable that's currently running. Without this guard,
# `rm -rf "$PORTABLE"` will silently delete the on-disk files of a live
# install — NTFS lets you unlink mapped exe/dll files, so the process
# keeps running on its mapped pages but its asset paths become a
# gravestone. Any subsequent code path that resolves an asset relative
# to current_exe (e.g. resolve_frontend_base_url's frontend/index.html
# check) then fails. See docs/retro/retro-portable-rm-running-install-2026-05-28.md.
if [ -d "$PORTABLE" ] && command -v powershell.exe >/dev/null 2>&1; then
    # Normalize to a backslashed path Get-Process can match against.
    portable_win=$(cygpath -w "$PORTABLE" 2>/dev/null || echo "$PORTABLE")
    running_pid=$(powershell.exe -NoProfile -Command "
        Get-Process -ErrorAction SilentlyContinue |
            Where-Object { \$_.Path -and \$_.Path -like '${portable_win}\\*' } |
            Select-Object -First 1 -ExpandProperty Id
    " 2>/dev/null | tr -d '\r\n ')
    if [ -n "$running_pid" ]; then
        echo "ERROR: a process (PID $running_pid) is running from $PORTABLE" >&2
        echo "       This should be impossible now that every local build gets a" >&2
        echo "       unique stamped folder — if you hit this, two builds collided" >&2
        echo "       on the same label. Pass an alternate output dir to proceed:" >&2
        echo "       task package -- ~/Desktop/staging" >&2
        exit 1
    fi
fi

# Clean previous
rm -rf "$PORTABLE" "$ARCHIVE_PATH"

# Create structure. No `data/` dir: portable builds are stateless on disk —
# user data lives in ~/.agentmux/versions/<version>/ (agentmux-common::
# RuntimeMode). A bundled data/ folder was vestigial (nothing wrote to it) and
# misled users into thinking their data lived next to the exe. See the README
# below and PR #1693.
mkdir -p "$PORTABLE/runtime/locales" "$PORTABLE/runtime/frontend" "$PORTABLE/runtime/tools/bin"

# Launcher in root
cp target/release/agentmux-launcher.exe "$PORTABLE/agentmux.exe"

# Portable marker — read by agentmux-common::RuntimeMode::current to
# distinguish portable extracts from installed builds (both ship a
# `runtime/` subdir, so the dir alone is not a discriminator) and by
# agentmux-cef read_build_label to show the ephemeral build label in the UI.
# Lives INSIDE runtime/ to keep the extract root clean (just agentmux.exe +
# README + runtime/). The launcher (at the root) looks one level down in
# runtime/, and the host/srv (which run FROM runtime/) find it next to
# themselves. Mac .app-bundle layouts (marker at the bundle root) still work.
printf 'AgentMux portable build %s\n' "$LABEL" > "$PORTABLE/runtime/agentmux-portable.marker"

# README
cat > "$PORTABLE/README.txt" <<READMEEOF
AgentMux $LABEL - Portable Edition

Quick Start:
  1. Extract this folder (or the .7z archive) anywhere.
     Windows 11 opens .7z itself; on Windows 10 use 7-Zip (https://7-zip.org).
  2. Run agentmux.exe

Requirements:
  - Windows 10/11 x64
  - No installation needed
  - No admin rights required

Data:
  Your data is NOT stored in this folder. AgentMux keeps it in your user
  profile, under a per-channel folder (this build's channel: ${CHANNEL}):

    %USERPROFILE%\\.agentmux\\channels\\${CHANNEL}\\
    (e.g. C:\\Users\\<you>\\.agentmux\\channels\\${CHANNEL}\\)

  Per-version (a separate folder per AgentMux version):
      versions\\${VERSION}\\data\\        session history and block state
      versions\\${VERSION}\\logs\\        host and sidecar logs
      versions\\${VERSION}\\cef-cache\\   browser cache (safe to delete when closed)

  Channel-wide (shared across versions, so settings and agents survive upgrades):
      config\\    settings.json, and widgets.json for your own pane tabs (optional)
      agents\\    agent working directories

  This makes the portable folder disposable: move it, re-extract it, or delete
  it without losing anything. A portable copy and an installed copy of the same
  version share this data, and your agents and sign-in carry across versions.
  To back up or transfer your data, copy the channel folder above - NOT this
  portable folder.
READMEEOF

# Runtime binaries — versioned filenames so WER dumps & Event Viewer show versions.
# Phase 3 sandbox (#1374): bootstrap.exe presence means sandbox feature was ON.
# bootstrap.exe becomes the host entry point (renamed); it loads the cdylib and
# passes real sandbox_info. Versioned names keep exe↔dll basenames matched so
# bootstrap resolves the right DLL. Without bootstrap.exe (--no-default-features)
# the raw bin runs the no_sandbox path directly.
if [ -f target/release/bootstrap.exe ]; then
    cp target/release/bootstrap.exe    "$PORTABLE/runtime/agentmux-$VERSION.exe"
    cp target/release/agentmux_cef.dll "$PORTABLE/runtime/agentmux-$VERSION.dll"
else
    echo "WARNING: no target/release/bootstrap.exe — packaging a NON-SANDBOX host (raw bin)." >&2
    echo "         Build with the default 'sandbox' feature for a sandboxed Windows portable." >&2
    cp target/release/agentmux-cef.exe "$PORTABLE/runtime/agentmux-$VERSION.exe"
fi

# Stamp the AgentMux icon onto the staged host exe. Under the Phase 3 sandbox the
# host is CEF's bootstrap.exe (ships Chrome's icon); winres can't edit an
# already-built PE, so rewrite the icon resource here (idempotent on the raw-bin
# path too). Fixes the Explorer / Task Manager / Alt-Tab exe-file icon — the
# #1633 regression. The running WINDOW icon is fixed separately at runtime
# (set_window_icon → WM_SETICON).
# The description names the build in Task Manager; local builds say so.
HOST_DESC="AgentMux v$VERSION"
[ "$CHANNEL" = "stable" ] || HOST_DESC="$HOST_DESC (local)"
bash "$REPO_ROOT/scripts/inject-exe-icon.sh" "$PORTABLE/runtime/agentmux-$VERSION.exe" "$HOST_DESC"

cp dist/bin/agentmux-srv-$VERSION-windows.x64.exe "$PORTABLE/runtime/"

# Streaming bash wrapper — invoked by Claude's Bash subprocess via the
# PreToolUse hook (agent_config.rs auto-injects .claude/hooks.json
# pointing at "agentmux-bashwrap hook"). agentmux-srv adds tools/bin
# to PATH for Claude's env, so the wrapper must land in tools/bin.
# See docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md.
cp target/release/agentmux-bashwrap.exe "$PORTABLE/runtime/tools/bin/"

# MCP server binary — auto-injected into .mcp.json (agent_config.rs
# "command": "agentmux-mcp"). agentmux-srv adds tools/bin to Claude's
# PATH so the binary is found without an absolute path.
# See docs/specs/SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md §5.2.
cp target/release/agentmux-mcp.exe "$PORTABLE/runtime/tools/bin/"

# Remote helpers — agentmux-remote for every SSH host platform, since any
# AgentMux may connect to any host; srv uploads the matching one to a host for
# file browsing and durable sessions. tools/remote/<target>/agentmux-remote.
# See docs/specs/SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md §9.1.
bash scripts/build-remote-helpers.sh dist/remote
if [ -d dist/remote ]; then
    mkdir -p "$PORTABLE/runtime/tools/remote"
    cp -R dist/remote/. "$PORTABLE/runtime/tools/remote/"
fi

# wsh has been retired — see docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md. No binary
# to ship anymore; AGENTMUX env var is now a plain "1" sentinel.

# Frontend
cp -r dist/frontend/* "$PORTABLE/runtime/frontend/"
# Licence texts the build writes (scripts/third-party-notices.mjs); every package
# must carry them.
if [ ! -f "$PORTABLE/runtime/frontend/licenses/THIRD_PARTY_NOTICES.txt" ]; then
    echo "ERROR: dist/frontend/licenses/THIRD_PARTY_NOTICES.txt is missing; rebuild the frontend (task build:frontend)." >&2
    exit 1
fi

# CEF core
cp dist/cef/libcef.dll "$PORTABLE/runtime/"
cp dist/cef/chrome_elf.dll "$PORTABLE/runtime/" 2>/dev/null || true
cp dist/cef/icudtl.dat "$PORTABLE/runtime/" 2>/dev/null || true
cp dist/cef/v8_context_snapshot.bin "$PORTABLE/runtime/" 2>/dev/null || true

# GPU support
cp dist/cef/libEGL.dll dist/cef/libGLESv2.dll dist/cef/d3dcompiler_47.dll "$PORTABLE/runtime/" 2>/dev/null || true
# Software-GL fallback (SwiftShader) — must reach runtime/ too, not just dist/cef.
# Without these the GPU degrades to the disabled DOM renderer when hardware GL
# can't boot. Paired with --enable-unsafe-swiftshader (app.rs) + the bundle step.
cp dist/cef/vk_swiftshader.dll dist/cef/vulkan-1.dll dist/cef/vk_swiftshader_icd.json "$PORTABLE/runtime/" 2>/dev/null || true

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
        actual=$(sha256sum < "$cache_key" | cut -d' ' -f1)
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
        actual=$(sha256sum < "$cache_key" | cut -d' ' -f1)
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
    "https://github.com/jqlang/jq/releases/download/jq-1.8.2/jq-windows-amd64.exe" \
    "a6fc67fedaf9128a3309a1e2ebb8b986aeccf70122ee46d2cb4849e423f0c627" \
    ""
bundle_tool \
    "rg.exe" \
    "https://github.com/BurntSushi/ripgrep/releases/download/15.2.0/ripgrep-15.2.0-x86_64-pc-windows-msvc.zip" \
    "71b2fef860abe467217a538ff31de02f5258807c0129f771846f87bd029aafc5" \
    "ripgrep-15.2.0-x86_64-pc-windows-msvc/rg.exe"

# Verify versions match. In a sandbox build the host exe is CEF's bootstrap.exe
# (no agentmux version baked in) — the agentmux version lives in the cdylib DLL
# (env!("CARGO_PKG_VERSION")) — so verify the DLL when it was staged, else the
# raw bin. (reagent P1 #1633.)
if [ -f "$PORTABLE/runtime/agentmux-$VERSION.dll" ]; then
    CEF_VER=$(grep -ao "$VERSION" "$PORTABLE/runtime/agentmux-$VERSION.dll" | head -1)
else
    CEF_VER=$(grep -ao "$VERSION" "$PORTABLE/runtime/agentmux-$VERSION.exe" | head -1)
fi
SRV_VER=$(grep -ao "$VERSION" "$PORTABLE/runtime/agentmux-srv-$VERSION-windows.x64.exe" | head -1)
if [ "$CEF_VER" != "$VERSION" ] || [ "$SRV_VER" != "$VERSION" ]; then
    echo "ERROR: Binary version mismatch! CEF=$CEF_VER SRV=$SRV_VER expected=$VERSION" >&2
    exit 1
fi

# Size
DIR_SIZE=$(du -sh "$PORTABLE" | cut -f1)

# Archive: a solid LZMA2 .7z (docs/specs/SPEC_WINDOWS_PORTABLE_7Z_2026_10_11.md).
# On 0.59.19 that is 127 MiB, against 205 MiB for the Deflate .zip it replaced.
#
# 7-Zip only: there is no fallback to another format. A fallback would ship a
# different file under a different name, and a missing archive (or the wrong
# one) is what this check exists to catch. winget and the 7-Zip installer
# don't put 7z on PATH, so its default install folder is looked in too.
find_7z() {
    local c
    for c in 7z 7za; do
        if command -v "$c" >/dev/null 2>&1; then
            command -v "$c"
            return 0
        fi
    done
    for c in "${PROGRAMFILES:-C:/Program Files}/7-Zip/7z.exe" "C:/Program Files/7-Zip/7z.exe"; do
        if [ -x "$c" ]; then
            echo "$c"
            return 0
        fi
    done
    return 1
}

cd "$OUTDIR"
ARCHIVE_NAME=$(basename "$ARCHIVE_PATH")
rm -f "$ARCHIVE_NAME"

if ! SEVENZIP=$(find_7z); then
    echo "ERROR: 7-Zip not found, so $ARCHIVE_NAME can't be made." >&2
    echo "       Install it (winget install 7zip.7zip) or put 7z on PATH, then re-run." >&2
    exit 1
fi

# Run from inside the folder so the launcher and runtime/ sit at the archive
# root, with no wrapping folder.
portable_basename=$(basename "$PORTABLE")
if ! (cd "$portable_basename" && "$SEVENZIP" a -t7z -m0=lzma2 -mx=9 -ms=on -mmt=on "../$ARCHIVE_NAME" . >/dev/null); then
    echo "ERROR: 7-Zip failed to create $ARCHIVE_NAME" >&2
    exit 1
fi
if ! "$SEVENZIP" t "$ARCHIVE_NAME" >/dev/null; then
    echo "ERROR: $ARCHIVE_NAME failed 7-Zip's integrity test" >&2
    exit 1
fi

ARCHIVE_SIZE=$(du -sh "$ARCHIVE_NAME" 2>/dev/null | cut -f1 || echo "N/A")

# Append compact size row to VERSION_HISTORY.md under the "## Sizes" table
# if it exists. REPO_ROOT was captured at the top of the script BEFORE the
# `cd "$OUTDIR"` above, so it still points at the repo root even though cwd
# is now the output directory. Skipped silently if the file / section
# isn't there.
#
# Skipped entirely for LOCAL labeled builds (AGENTMUX_BUILD_LABEL set):
# writing to the git-tracked VERSION_HISTORY.md on every smoke build would
# reintroduce the exact git-mutation-per-build problem this whole scheme
# exists to eliminate. The size table is release bookkeeping; only the
# release flow should touch it. (SPEC_LOCAL_BUILD_VERSIONING_2026_05_28.md)
if [ -z "${AGENTMUX_BUILD_LABEL:-}" ] \
    && [ -f "$REPO_ROOT/VERSION_HISTORY.md" ] \
    && grep -q "^## Sizes " "$REPO_ROOT/VERSION_HISTORY.md" 2>/dev/null; then
    DIR_BYTES=$(find "$PORTABLE" -type f -printf '%s\n' 2>/dev/null | awk '{s+=$1} END {print s+0}')
    ARCHIVE_BYTES=$(stat -c '%s' "$ARCHIVE_NAME" 2>/dev/null || echo 0)
    DIR_MIB=$(awk "BEGIN {printf \"%.1f\", $DIR_BYTES/1024/1024}")
    ARCHIVE_MIB=$(awk "BEGIN {printf \"%.1f\", $ARCHIVE_BYTES/1024/1024}")
    TODAY=$(date +%Y-%m-%d)
    # The table's "ZIP" column holds the archive's size, now a .7z.
    ROW="| $VERSION | $TODAY | $ARCHIVE_MIB MiB | $DIR_MIB MiB | |"

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
echo "[SUCCESS] Portable $LABEL"
echo "  Directory: $PORTABLE ($DIR_SIZE)"
echo "  Archive: $ARCHIVE_PATH ($ARCHIVE_SIZE)"
