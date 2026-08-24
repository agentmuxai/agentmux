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

# Verify required build artifacts exist
require() {
    if [ ! -e "$1" ]; then
        echo "ERROR: required artifact $1 missing — run \`task build:host && task build:backend && task build:frontend && task bundle && task copy:schema\` first" >&2
        exit 1
    fi
}
require dist/cef/agentmux-cef
require dist/cef/agentmux-launcher
require dist/cef/libcef.so
require dist/bin/agentmux-srv-${VERSION}-linux.x64
require target/release/agentmux-mcp
# `task build:frontend` outputs to dist/frontend (per vite.config.ts outDir).
# `dev:serve` symlinks dist/cef/frontend → ../frontend for the host's runtime
# lookup, but that symlink isn't created by `task bundle` — and on a clean
# checkout running `task package:linux` it doesn't exist. Read straight from
# the build output to keep packaging reproducible from a fresh checkout.
require dist/frontend/index.html

# --- Release gate: the bundled libcef.so MUST carry the BeginWindowDrag patch,
#     or left-click window drag silently no-ops in the shipped AppImage (the
#     runtime ABI guard only surfaces it after the user clicks). The symbol check
#     needs an UNSTRIPPED .so: dist/cef/libcef.so is unstripped at this point in the
#     canonical `task package:linux` flow (bundle:linux copies the build-tree output;
#     the strip below runs on the AppDir copy). If dist/cef was pre-stripped
#     out-of-band, fall back to verifying the resolved build-tree source. This is
#     release-only — `task dev`/`task bundle` never run this script. Override with
#     AGENTMUX_SKIP_CEF_PATCH_CHECK=1 (emergency only). ---
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

echo "Building AgentMux v$VERSION AppImage → $OUTPUT"

# --- 1. Wipe and recreate AppDir ---
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin/locales"
mkdir -p "$APPDIR/usr/share/icons/hicolor"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/assets"

# --- 2. Host binary (keep cargo name agentmux-cef so the launcher's
#        find_cef_binary final fallback resolves it without a launcher
#        code change — see SPEC §3 step 1a) ---
cp dist/cef/agentmux-cef "$APPDIR/usr/bin/agentmux-cef"

# --- 2b. Launcher binary — the AppRun's exec target. The launcher
#         supervises srv + host as a process group (A0). Without this
#         the AppImage host binary runs alone; every launcher_ipc
#         report_* call from the host silently no-ops. ---
cp dist/cef/agentmux-launcher "$APPDIR/usr/bin/agentmux-launcher"

# --- 3. Backend sidecar (versioned filename — host's resolve_backend_binary
#        looks for `agentmux-srv-<VERSION>-linux.x64` next to the host) ---
cp "dist/bin/agentmux-srv-${VERSION}-linux.x64" "$APPDIR/usr/bin/"

# --- 3b. Bundled tools — agentmux-srv adds <exe_dir>/tools/bin to Claude's PATH.
#         On Linux, exe_dir = usr/bin, so tools land at usr/bin/tools/bin/.
#         agentmux-mcp is the Shell MCP server; without it the Shell tool
#         fails with command-not-found on packaged builds. ---
mkdir -p "$APPDIR/usr/bin/tools/bin"
cp target/release/agentmux-mcp "$APPDIR/usr/bin/tools/bin/agentmux-mcp"

# --- 4. CEF runtime (libcef.so, GL libs, paks, snapshots, sandbox) ---
for f in libcef.so libEGL.so libGLESv2.so chrome-sandbox chrome_crashpad_handler \
         icudtl.dat snapshot_blob.bin v8_context_snapshot.bin \
         chrome_100_percent.pak chrome_200_percent.pak resources.pak \
         headless_command_resources.pak \
         libvk_swiftshader.so vk_swiftshader_icd.json libvulkan.so.1; do
    if [ -f "dist/cef/$f" ]; then
        cp "dist/cef/$f" "$APPDIR/usr/bin/"
    fi
done

# Strip the unstripped libcef.so + GL libs to halve their size. The chromium
# build emits libcef.so at 613MB with full .symtab/.strtab — fine for dev
# debugging in dist/cef/ but huge for distribution. `strip` removes the local
# (non-dynamic) symbol table but keeps .dynsym so dlopen + relocations still
# work; saves ~210MB on libcef.so alone (~33% AppImage reduction).
# libvk_swiftshader.so + libvulkan.so.1 added for CEF 148 — same treatment.
for so in libcef.so libEGL.so libGLESv2.so libvk_swiftshader.so libvulkan.so.1; do
    if [ -f "$APPDIR/usr/bin/$so" ]; then
        strip "$APPDIR/usr/bin/$so"
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

# Strip frontend source maps from the release artifact (debug-only, ~28 MB).
# Mirrors the STRIP_MAPS policy for release builds
# (docs/specs/SPEC_PORTABLE_SOURCE_MAPS_2026_06_01.md): the app runs identically;
# only prod-stack-trace symbolication is lost.
_maps=$(find "$APPDIR/usr/bin/frontend" -name '*.map' | wc -l)
if [ "$_maps" -gt 0 ]; then
    find "$APPDIR/usr/bin/frontend" -name '*.map' -delete
    echo "Stripped $_maps source-map file(s) from the AppImage frontend"
fi

# --- 6. Schema (optional — only present if `task copy:schema` ran) ---
if [ -d dist/schema ]; then
    mkdir -p "$APPDIR/usr/share/agentmux"
    cp -r dist/schema "$APPDIR/usr/share/agentmux/"
fi

# --- 6b. VERSION marker — read by AppRun to key the extract-once-cache.
#         AppRun re-execs from $HOME/.local/share/agentmux/extracted/<VERSION>/
#         on second+ launches to skip SquashFS decompression. Spec:
#         docs/specs/linux-appimage-cold-launch-tax-2026-05-08.md (Phase 2).
mkdir -p "$APPDIR/usr/share/agentmux"
echo "$VERSION" > "$APPDIR/usr/share/agentmux/VERSION"

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
