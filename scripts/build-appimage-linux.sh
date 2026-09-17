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
#     install-userns-apparmor-fix.sh  → pkexec-invoked helper installing the
#                                       AppArmor userns exception (see
#                                       docs/specs/SPEC_LINUX_SANDBOX_APPARMOR_USERNS_2026_08_23.md)
#     assets/linux/...                → source-of-truth tree the installer reads
#     usr/bin/agentmux-launcher       → entry point execed by AppRun; supervises srv + host
#     usr/bin/agentmux-cef             → CEF host binary; launcher's find_cef_binary final fallback
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
# Local builds use the label (e.g. 0.49.2+g3f1a2bc.dirty.20260625T1040.12345)
# so each AppImage has a unique filename and is identifiable. Release builds
# (RELEASE_CHANNEL=stable, no AGENTMUX_BUILD_LABEL) use just the version.
if [ -n "${AGENTMUX_BUILD_LABEL:-}" ]; then
    OUTPUT="$OUTDIR/AgentMux_${AGENTMUX_BUILD_LABEL}_amd64.AppImage"
else
    OUTPUT="$OUTDIR/AgentMux_${VERSION}_amd64.AppImage"
fi

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

# --- 0. Wipe the whole AppDir first (ReAgent P1, PR #3236) — the original
#        script did `rm -rf "$APPDIR"` before staging anything, which this
#        refactor initially dropped: stage-linux-runtime.sh only wipes its
#        own $STAGING_ROOT/usr, never the AppDir's top-level content
#        (assets/linux/, install-linux-desktop.sh,
#        install-userns-apparmor-fix.sh, agentmux.desktop, agentmux.png,
#        .DirIcon — all populated via `cp`, i.e. overwrite-only). Without
#        this, a removed/renamed source file would silently leave a stale
#        copy in every subsequent local build's shipped AppImage. ---
rm -rf "$APPDIR"

# --- 1-6b. Shared runtime staging (binaries, CEF libs, frontend, schema,
#           VERSION marker) — extracted into stage-linux-runtime.sh so
#           build-deb-linux.sh and build-tarball-linux.sh ship the identical
#           runtime instead of a hand-copied subset. This call also performs
#           the required-artifact checks and the BeginWindowDrag release gate
#           that used to be inlined here. ---
bash "$REPO_ROOT/scripts/stage-linux-runtime.sh" "$APPDIR"

# --- Build stamp for the AppRun extract-once cache ---------------------
# The cache used to be keyed on VERSION alone. `task package` deliberately
# does NOT bump the version, so every local build of a given version shared
# one extraction dir and the FIRST one extracted won forever: later builds
# silently re-exec'd the older binary, taking its baked per-build channel
# with it. Two local 0.56.3 builds reproduced it — the second launch ran the
# first's binary and first's channel. Keyed on the build label instead, which
# is unique per build. Release builds don't set AGENTMUX_BUILD_LABEL, so they
# fall back to VERSION and keep one cache per released version, as before.
if [ -n "${AGENTMUX_BUILD_LABEL:-}" ]; then
    mkdir -p "$APPDIR/usr/share/agentmux"
    printf '%s' "$AGENTMUX_BUILD_LABEL" | tr -c 'A-Za-z0-9._+-' '_' \
        > "$APPDIR/usr/share/agentmux/BUILD_ID"
fi
mkdir -p "$APPDIR/usr/share/icons/hicolor"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/assets"

echo "Building AgentMux v$VERSION AppImage → $OUTPUT"

# --- 7. AppRun + helper script + assets the installer reads ---
cp scripts/linux-apprun.sh "$APPDIR/AppRun"
chmod +x "$APPDIR/AppRun"
cp scripts/install-linux-desktop.sh "$APPDIR/install-linux-desktop.sh"
chmod +x "$APPDIR/install-linux-desktop.sh"
# Privileged one-time helper for the AppArmor userns-restriction fix
# (docs/specs/SPEC_LINUX_SANDBOX_APPARMOR_USERNS_2026_08_23.md) — invoked
# via pkexec from agentmux-cef's linux_sandbox::run_pkexec_fix(), which
# resolves this path via $APPDIR (same top-level-of-AppDir convention as
# install-linux-desktop.sh above, not a subdirectory).
cp scripts/install-userns-apparmor-fix.sh "$APPDIR/install-userns-apparmor-fix.sh"
chmod +x "$APPDIR/install-userns-apparmor-fix.sh"
# install-linux-desktop.sh resolves REPO_ROOT as `<script_dir>/..`. Inside
# the AppImage the script lives at AppDir/install-linux-desktop.sh, so its
# REPO_ROOT becomes AppDir. The script then reads assets/linux/... → place
# the assets at AppDir/assets/linux/ to satisfy that path.
cp -r assets/linux "$APPDIR/assets/"

# --- 8. Top-level desktop file (required by appimagetool) ---
# appimagetool wants Exec=AppRun (relative); the user-installed copy gets
# Exec=$APPIMAGE substituted at runtime by install-linux-desktop.sh.
# StartupWMClass must match the app_id this build's binary will advertise
# (window_settings.rs::linux_app_id() = agentmux-<channel>-<version>) so
# desktop-integration tools that read this file directly (not the
# runtime-installed copy) still resolve the right icon.
cp assets/linux/agentmux.desktop "$APPDIR/agentmux.desktop"
sed -i 's|^Exec=.*|Exec=AppRun %F|' "$APPDIR/agentmux.desktop"
BUILD_CHANNEL="${AGENTMUX_BUILD_CHANNEL_DEFAULT:-stable}"
sed -i "s|__WMCLASS__|agentmux-${BUILD_CHANNEL}-${VERSION}|" "$APPDIR/agentmux.desktop"

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
# appimagetool's bundled mksquashfs only ships the zstd compressor (no xz), so
# crank zstd to its max level (22 vs the default 15). The bulk of the image is
# libcef.so (~414 MB); the higher level trades build time for a smaller artifact.
# Decompress cost is absorbed by AppRun's extract-once-cache on first launch.
mkdir -p "$OUTDIR"
rm -f "$OUTPUT"
ARCH=x86_64 "$APPIMAGETOOL" --no-appstream \
    --comp zstd --mksquashfs-opt -Xcompression-level --mksquashfs-opt 22 \
    "$APPDIR" "$OUTPUT"

chmod +x "$OUTPUT"
echo ""
echo "✓ Built AppImage: $OUTPUT"
ls -lh "$OUTPUT"
