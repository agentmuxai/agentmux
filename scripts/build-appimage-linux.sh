#!/usr/bin/env bash
# Build AgentMux as a portable AppImage on Linux x86_64.
#
# Usage:
#   bash scripts/build-appimage-linux.sh [output-dir]
#
#   output-dir defaults to ~/Desktop. The AppImage is named
#   AgentMux_{version}_amd64.AppImage in that directory.
#
# Prerequisites (one-time):
#   - appimagetool on PATH (or at ~/.local/bin/appimagetool).
#   - dist/cef/agentmux-cef + libcef.so + paks (run `task build:host && task bundle`)
#   - dist/bin/agentmux-srv-{version}-linux.x64 (run `task build:backend`)
#   - dist/cef/frontend/index.html (run `task build:frontend`)
#
# Layout the AppImage runtime expects:
#   AppDir/
#     AppRun                          → scripts/linux-apprun.sh (sets env, execs binary)
#     agentmux.desktop                → top-level entry point for appimagetool
#     agentmux.png                    → top-level icon
#     .DirIcon                        → REAL FILE COPY (not symlink — appimagetool's
#                                       default symlink breaks on other machines;
#                                       see CLAUDE.md "AppImage .DirIcon" memory)
#     install-linux-desktop.sh        → invoked by AppRun on first run to register
#                                       the user's desktop entry + hicolor icons
#     assets/linux/...                → source-of-truth tree the installer reads
#     usr/bin/agentmux                → the host binary (renamed from agentmux-cef)
#     usr/bin/agentmux-srv-X.Y.Z-...  → backend sidecar
#     usr/bin/libcef.so + paks + ...  → CEF runtime, colocated per CEF convention
#     usr/bin/frontend/index.html     → bundled frontend (served by IPC HTTP server)
#     usr/share/icons/hicolor/.../    → standard icon-theme tree for desktop integration

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(node -p "require('./package.json').version")"
OUTDIR="${1:-$HOME/Desktop}"
APPDIR="$REPO_ROOT/build/AgentMux.AppDir"
OUTPUT="$OUTDIR/AgentMux_${VERSION}_amd64.AppImage"

# Resolve appimagetool
APPIMAGETOOL="${APPIMAGETOOL:-}"
if [ -z "$APPIMAGETOOL" ]; then
    if command -v appimagetool >/dev/null 2>&1; then
        APPIMAGETOOL="$(command -v appimagetool)"
    elif [ -x "$HOME/.local/bin/appimagetool" ]; then
        APPIMAGETOOL="$HOME/.local/bin/appimagetool"
    else
        echo "ERROR: appimagetool not found on PATH or ~/.local/bin/appimagetool" >&2
        echo "       install: download from https://github.com/AppImage/appimagetool/releases" >&2
        exit 1
    fi
fi

# Verify required build artifacts exist
require() {
    if [ ! -e "$1" ]; then
        echo "ERROR: required artifact $1 missing — run \`task build:host && task build:backend && task build:frontend && task bundle && task copy:schema\` first" >&2
        exit 1
    fi
}
require dist/cef/agentmux-cef
require dist/cef/libcef.so
require dist/bin/agentmux-srv-${VERSION}-linux.x64
# `task build:frontend` outputs to dist/frontend (per vite.config.ts outDir).
# `dev:serve` symlinks dist/cef/frontend → ../frontend for the host's runtime
# lookup, but that symlink isn't created by `task bundle` — and on a clean
# checkout running `task package:linux` it doesn't exist. Read straight from
# the build output to keep packaging reproducible from a fresh checkout.
require dist/frontend/index.html

echo "Building AgentMux v$VERSION AppImage → $OUTPUT"

# --- 1. Wipe and recreate AppDir ---
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin/locales"
mkdir -p "$APPDIR/usr/share/icons/hicolor"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/assets"

# --- 2. Host binary (rename agentmux-cef → agentmux) ---
cp dist/cef/agentmux-cef "$APPDIR/usr/bin/agentmux"

# --- 3. Backend sidecar (versioned filename — host's resolve_backend_binary
#        looks for `agentmux-srv-<VERSION>-linux.x64` next to the host) ---
cp "dist/bin/agentmux-srv-${VERSION}-linux.x64" "$APPDIR/usr/bin/"

# --- 4. CEF runtime (libcef.so, GL libs, paks, snapshots, sandbox) ---
for f in libcef.so libEGL.so libGLESv2.so chrome-sandbox chrome_crashpad_handler \
         icudtl.dat snapshot_blob.bin v8_context_snapshot.bin \
         chrome_100_percent.pak chrome_200_percent.pak resources.pak; do
    if [ -f "dist/cef/$f" ]; then
        cp "dist/cef/$f" "$APPDIR/usr/bin/"
    fi
done
# Locales
if [ -d dist/cef/locales ]; then
    cp -r dist/cef/locales/. "$APPDIR/usr/bin/locales/"
fi

# --- 5. Frontend — copy from the canonical `task build:frontend` output
#        (dist/frontend) into the AppImage at usr/bin/frontend, the path
#        agentmux-cef looks for next to its binary. ---
cp -r dist/frontend "$APPDIR/usr/bin/frontend"

# --- 6. Schema (optional — only present if `task copy:schema` ran) ---
if [ -d dist/schema ]; then
    mkdir -p "$APPDIR/usr/share/agentmux"
    cp -r dist/schema "$APPDIR/usr/share/agentmux/"
fi

# --- 7. AppRun + helper script + assets the installer reads ---
cp scripts/linux-apprun.sh "$APPDIR/AppRun"
chmod +x "$APPDIR/AppRun"
cp scripts/install-linux-desktop.sh "$APPDIR/install-linux-desktop.sh"
chmod +x "$APPDIR/install-linux-desktop.sh"
# install-linux-desktop.sh resolves REPO_ROOT as `<script_dir>/..`. Inside
# the AppImage the script lives at AppDir/install-linux-desktop.sh, so its
# REPO_ROOT becomes AppDir. The script then reads assets/linux/... → place
# the assets at AppDir/assets/linux/ to satisfy that path.
cp -r assets/linux "$APPDIR/assets/"

# --- 8. Top-level desktop file (required by appimagetool) ---
# appimagetool wants Exec=AppRun (relative); the user-installed copy gets
# Exec=$APPIMAGE substituted at runtime by install-linux-desktop.sh.
cp assets/linux/agentmux.desktop "$APPDIR/agentmux.desktop"
sed -i 's|^Exec=.*|Exec=AppRun %F|' "$APPDIR/agentmux.desktop"

# --- 9. Top-level icon + .DirIcon (REAL COPY, not symlink — appimagetool's
#        default creates an absolute symlink that's broken outside this build
#        tree, causing Nautilus to show a generic icon for the AppImage file
#        itself; see CLAUDE.md "AppImage .DirIcon" memory) ---
cp assets/linux/icons/hicolor/256x256/apps/agentmux.png "$APPDIR/agentmux.png"
cp assets/linux/icons/hicolor/256x256/apps/agentmux.png "$APPDIR/.DirIcon"

# --- 10. Hicolor icon theme tree (for desktop integration once installed) ---
for size in 16 32 48 64 128 256 512; do
    src="assets/linux/icons/hicolor/${size}x${size}/apps/agentmux.png"
    dst="$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps/agentmux.png"
    if [ -f "$src" ]; then
        mkdir -p "$(dirname "$dst")"
        cp "$src" "$dst"
    fi
done

# --- 11. Build the AppImage ---
mkdir -p "$OUTDIR"
rm -f "$OUTPUT"
ARCH=x86_64 "$APPIMAGETOOL" --no-appstream "$APPDIR" "$OUTPUT"

chmod +x "$OUTPUT"
echo ""
echo "✓ Built AppImage: $OUTPUT"
ls -lh "$OUTPUT"
