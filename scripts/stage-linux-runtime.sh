#!/usr/bin/env bash
# Stage a runnable AgentMux Linux tree into a `usr/`-rooted directory.
#
# Shared by build-appimage-linux.sh, build-deb-linux.sh, build-tarball-linux.sh
# — extracted from build-appimage-linux.sh's original steps 1-6b so every
# Linux package format ships the identical runtime, not a hand-copied subset.
# See docs/specs/SPEC_LINUX_DISTRO_TARGETS_AND_DOWNLOADS_PAGE_2026_09_15.md §2.1
# (Codex review on PR #3234 caught an earlier draft of that spec assuming
# `dist/` itself — the flat build-output directory — was a runnable package
# input; it isn't. This script IS the real "given dist/ + target/, produce a
# runnable usr/-rooted tree" step that was missing.)
#
# Usage:
#   bash scripts/stage-linux-runtime.sh <staging-root>
#
#   <staging-root> becomes the `usr/` parent — this script populates
#   <staging-root>/usr/bin/... and <staging-root>/usr/share/agentmux/...
#   Callers create <staging-root> however their format needs it laid out
#   (an AppDir, a .deb package root, a portable-tarball directory) and add
#   their own format-specific entry points / metadata on top.
#
# Prerequisites (one-time, same as build-appimage-linux.sh always needed):
#   - dist/cef/agentmux-cef + libcef.so + paks   (task build:host && task bundle)
#   - dist/bin/agentmux-srv-{version}-linux.x64  (task build:backend)
#   - dist/cef/frontend/index.html                (task build:frontend, via dist/frontend)
#   - target/release/agentmux-mcp                 (cargo build --release -p agentmux-mcp,
#                                                   bundled by `task bundle`)
#
# Does NOT populate anything format-specific: no AppRun, no .desktop file, no
# icons, no assets/linux/, no DEBIAN/control. Callers own all of that.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

STAGING_ROOT="${1:?Usage: stage-linux-runtime.sh <staging-root>}"
VERSION="$(node -p "require('./package.json').version")"

require() {
    if [ ! -e "$1" ]; then
        echo "ERROR: required artifact $1 missing — run \`task build:host && task build:backend && task build:frontend && task bundle && task copy:schema\` first" >&2
        exit 1
    fi
}
require dist/cef/agentmux-cef
require dist/cef/agentmux-launcher
require dist/cef/libcef.so
require "dist/bin/agentmux-srv-${VERSION}-linux.x64"
require target/release/agentmux-mcp
require dist/frontend/index.html

# --- Release gate: the bundled libcef.so MUST carry the BeginWindowDrag patch,
#     or left-click window drag silently no-ops in the shipped package (the
#     runtime ABI guard only surfaces it after the user clicks). Applies to
#     every packaged format equally — not an AppImage-only concern — so it
#     lives here now instead of being duplicated per format. See the original
#     comment in build-appimage-linux.sh's git history for the full rationale.
#     Override with AGENTMUX_SKIP_CEF_PATCH_CHECK=1 (emergency only).
if [ "${AGENTMUX_SKIP_CEF_PATCH_CHECK:-0}" != "1" ]; then
    set +e
    bash "$REPO_ROOT/scripts/verify-cef-patch.sh" dist/cef/libcef.so
    patch_rc=$?
    if [ "$patch_rc" = "2" ]; then
        echo "→ dist/cef/libcef.so couldn't be verified (stripped?); checking the resolved source…" >&2
        cef_src="$(bash "$REPO_ROOT/scripts/resolve-cef-runtime.sh" 2>/dev/null || true)"
        if [ -n "$cef_src" ]; then
            bash "$REPO_ROOT/scripts/verify-cef-patch.sh" "$cef_src"
            patch_rc=$?
        fi
    fi
    set -e
    case "$patch_rc" in
        0) echo "✓ libcef.so carries the BeginWindowDrag patch" ;;
        1) echo "ERROR: bundled libcef.so lacks the BeginWindowDrag patch — refusing to" >&2
           echo "       package a release with broken left-click window drag. Build the" >&2
           echo "       patched libcef (docs/cef-build/build-patched-libcef.md) or set" >&2
           echo "       AGENTMUX_SKIP_CEF_PATCH_CHECK=1 to override." >&2
           exit 1 ;;
        *) echo "WARNING: could not verify the BeginWindowDrag patch (stripped runtime and" >&2
           echo "         no unstripped source to check). Proceeding — verify drag manually." >&2 ;;
    esac
fi

echo "Staging AgentMux v$VERSION runtime → $STAGING_ROOT/usr/bin"

# --- 1. Wipe and recreate the usr/ tree ---
rm -rf "$STAGING_ROOT/usr"
mkdir -p "$STAGING_ROOT/usr/bin/locales"
mkdir -p "$STAGING_ROOT/usr/bin/tools/bin"

# --- 2. Host binary (keep cargo name agentmux-cef so the launcher's
#        find_cef_binary final fallback resolves it without a launcher
#        code change) ---
cp dist/cef/agentmux-cef "$STAGING_ROOT/usr/bin/agentmux-cef"

# --- 2b. Launcher binary — every format's entry point execs this; it
#         supervises srv + host as a process group (A0). Without it the
#         host binary runs alone and every launcher_ipc report_* call
#         from the host silently no-ops. ---
cp dist/cef/agentmux-launcher "$STAGING_ROOT/usr/bin/agentmux-launcher"

# --- 3. Backend sidecar (versioned filename — host's resolve_backend_binary
#        looks for `agentmux-srv-<VERSION>-linux.x64` next to the host) ---
cp "dist/bin/agentmux-srv-${VERSION}-linux.x64" "$STAGING_ROOT/usr/bin/"

# --- 3b. Bundled tools — agentmux-srv adds <exe_dir>/tools/bin to Claude's PATH.
#         exe_dir = usr/bin for every format staged this way, so tools land at
#         usr/bin/tools/bin/. agentmux-mcp is the Shell MCP server; without it
#         the Shell tool fails with command-not-found on packaged builds. ---
cp target/release/agentmux-mcp "$STAGING_ROOT/usr/bin/tools/bin/agentmux-mcp"

# --- 4. CEF runtime (libcef.so, GL libs, paks, snapshots, sandbox) ---
for f in libcef.so libEGL.so libGLESv2.so chrome-sandbox chrome_crashpad_handler \
         icudtl.dat snapshot_blob.bin v8_context_snapshot.bin \
         chrome_100_percent.pak chrome_200_percent.pak resources.pak \
         headless_command_resources.pak \
         libvk_swiftshader.so vk_swiftshader_icd.json libvulkan.so.1; do
    if [ -f "dist/cef/$f" ]; then
        cp "dist/cef/$f" "$STAGING_ROOT/usr/bin/"
    fi
done

# Strip the unstripped libcef.so + GL libs to halve their size — same
# treatment build-appimage-linux.sh always applied, now shared so every
# format benefits equally rather than only the format someone remembered to
# add it to.
for so in libcef.so libEGL.so libGLESv2.so libvk_swiftshader.so libvulkan.so.1; do
    if [ -f "$STAGING_ROOT/usr/bin/$so" ]; then
        strip "$STAGING_ROOT/usr/bin/$so"
    fi
done

# Locales
if [ -d dist/cef/locales ]; then
    cp -r dist/cef/locales/. "$STAGING_ROOT/usr/bin/locales/"
fi

# --- 5. Frontend — copy from the canonical `task build:frontend` output
#        (dist/frontend) into usr/bin/frontend, the path agentmux-cef looks
#        for next to its binary. ---
cp -r dist/frontend "$STAGING_ROOT/usr/bin/frontend"

# Strip frontend source maps from the release artifact (debug-only, ~28 MB).
# Mirrors the STRIP_MAPS policy for release builds
# (docs/specs/SPEC_PORTABLE_SOURCE_MAPS_2026_06_01.md): the app runs identically;
# only prod-stack-trace symbolication is lost.
_maps=$(find "$STAGING_ROOT/usr/bin/frontend" -name '*.map' | wc -l)
if [ "$_maps" -gt 0 ]; then
    find "$STAGING_ROOT/usr/bin/frontend" -name '*.map' -delete
    echo "Stripped $_maps source-map file(s) from the staged runtime"
fi

# --- 6. Schema (optional — only present if `task copy:schema` ran) ---
if [ -d dist/schema ]; then
    mkdir -p "$STAGING_ROOT/usr/share/agentmux"
    cp -r dist/schema "$STAGING_ROOT/usr/share/agentmux/"
fi

# --- 6b. VERSION marker — same convention the AppImage's AppRun reads for
#         its extract-once-cache key. Harmless for formats that don't use
#         it; kept so every staged tree looks identical regardless of which
#         format ultimately wraps it. ---
mkdir -p "$STAGING_ROOT/usr/share/agentmux"
echo "$VERSION" > "$STAGING_ROOT/usr/share/agentmux/VERSION"

echo "✓ Staged runtime at $STAGING_ROOT/usr/bin"
