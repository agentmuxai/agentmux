#!/usr/bin/env bash
# Build AgentMux as a signed macOS .app + .dmg (Developer ID, hardened runtime).
#
# Usage:  bash scripts/package-macos.sh [output-dir]   (output-dir defaults to ~/Desktop)
#
# Prerequisites (the `task package:macos` deps run these for you):
#   task build:host      → dist/cef/agentmux-cef
#   task build:backend   → dist/bin/agentmux-srv-<VERSION>-darwin.arm64
#   task build:frontend  → dist/frontend/index.html
#   task bundle          → dist/Frameworks/Chromium Embedded Framework.framework + GL libs in dist/cef/
#
# Bundle layout. The host resolves everything RELATIVE TO ITS OWN BINARY
# (current_exe().parent()), so this needs ZERO Rust changes — it mirrors the
# Linux AppImage's "next to the binary" convention, mapped onto .app dirs:
#
#   AgentMux.app/Contents/
#     Info.plist
#     MacOS/
#       agentmux-launcher                     ← CFBundleExecutable: tiny binary
#                                                that paints the splash instantly,
#                                                then spawns + supervises srv + host
#       agentmux-cef                          ← host (spawned by the launcher;
#                                                re-execs for renderer/gpu helpers)
#       agentmux-srv-<VERSION>-darwin.arm64   ← backend (sidecar::resolve_backend_binary)
#       frontend/                             ← bundled UI (resolve_frontend_base_url)
#       *.dylib + vk_swiftshader_icd.json     ← GL libs (Chromium DIR_MODULE = exe dir)
#     Frameworks/
#       Chromium Embedded Framework.framework ← cef-rs ../Frameworks/... lookup +
#                                                CefSettings framework_dir_path
#     Resources/
#       AgentMux.icns
#
# Signing/notarization: each Mach-O is signed inside-out with --options runtime.
# Notarization is ATTEMPTED via the `notarytool` keychain profile; if it fails
# (e.g. an expired Apple Developer agreement → HTTP 403) the script still emits a
# SIGNED — but un-notarized — DMG and warns. Gatekeeper will then require a
# right-click→Open on first launch on other Macs until the build is notarized.
#   NOTARIZE=0            skip notarization entirely (signed-only)
#   NOTARY_PROFILE=name   keychain profile name (default: notarytool)
#   MACOS_SIGN_CERT="..." override the Developer ID cert name
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(node -p "require('./package.json').version")"
ARCH="arm64"   # Apple Silicon; cef-dll-sys resolves the aarch64 framework
OUTDIR="${1:-$HOME/Desktop}"
APP="$REPO_ROOT/build/AgentMux.app"
DMG="$OUTDIR/AgentMux_${VERSION}_${ARCH}.dmg"
# Resolve the Developer ID cert from the keychain — never hardcode the signing
# identity (name + team id) in the public repo. Override with MACOS_SIGN_CERT.
# Credential/identity details live in the private agentmux-builder repo.
CERT="${MACOS_SIGN_CERT:-$(security find-identity -v -p codesigning 2>/dev/null \
        | awk -F'"' '/Developer ID Application/ {print $2; exit}')}"
[ -n "$CERT" ] || { echo "❌ No 'Developer ID Application' identity in the keychain (and MACOS_SIGN_CERT unset)." >&2; exit 1; }
ENTITLEMENTS="$REPO_ROOT/build/entitlements.mac.plist"
# Bundle id: ai.agentmux.<channel>.<version>
# Channel matches AGENTMUX_BUILD_CHANNEL_DEFAULT compiled into the binaries
# (agentmux-common/src/data_paths.rs; default "stable"), keeping OS identity and
# runtime channel in sync. Version suffix makes every release a distinct macOS app,
# so double-clicking any build works without needing `open -n`.
# Sanitize channel to the bundle-id charset [A-Za-z0-9.-]; VERSION (semver) is
# already clean.
# See docs/specs/SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md.
CHANNEL="${AGENTMUX_BUILD_CHANNEL_DEFAULT:-stable}"
CHANNEL_ID="$(printf '%s' "$CHANNEL" | tr -C 'A-Za-z0-9.-' '-' | tr '[:upper:]' '[:lower:]')"
BUNDLE_ID="ai.agentmux.${CHANNEL_ID}.${VERSION}"
NOTARIZE="${NOTARIZE:-1}"
NOTARY_PROFILE="${NOTARY_PROFILE:-notarytool}"
SRV="dist/bin/agentmux-srv-${VERSION}-darwin.${ARCH}"

require() { [ -e "$1" ] || { echo "❌ missing required artifact: $1 — run the build steps first" >&2; exit 1; }; }
require dist/cef/agentmux-cef
require dist/cef/agentmux-launcher
require target/release/agentmux-mcp
require "$SRV"
require dist/frontend/index.html
require dist/schema/settings.json
require "dist/Frameworks/Chromium Embedded Framework.framework"
[ -f "$ENTITLEMENTS" ] || { echo "❌ missing entitlements: $ENTITLEMENTS" >&2; exit 1; }

# ── Hard BeginWindowDrag-patch gate ───────────────────────────────────────────
# Release builds MUST ship the patched CEF framework (agentmuxai/cef fork) — the
# upstream cef-dll-sys framework lacks CefWindow::BeginWindowDrag(), so native
# window drag / floating-pane resize silently no-ops. This is the macOS analogue
# of the Linux gate in build-appimage-linux.sh. It runs on dist/Frameworks/ —
# UNSTRIPPED at this point; the strip (which removes the local patch symbol) runs
# later, inside the assembled .app. Escape hatch: AGENTMUX_SKIP_CEF_PATCH_CHECK=1
# for a deliberate upstream-CEF package (e.g. local smoke test without the patch).
if [ "${AGENTMUX_SKIP_CEF_PATCH_CHECK:-0}" = "1" ]; then
    echo "⚠️  AGENTMUX_SKIP_CEF_PATCH_CHECK=1 — skipping the BeginWindowDrag patch gate" >&2
else
    if ! bash scripts/verify-cef-framework-darwin.sh "dist/Frameworks/Chromium Embedded Framework.framework"; then
        echo "❌ CEF framework patch gate failed — refusing to package an unpatched/unverifiable" >&2
        echo "   framework. Point AGENTMUX_CEF_RUNTIME_DIR_DARWIN at the patched framework and" >&2
        echo "   re-run task bundle:darwin, or set AGENTMUX_SKIP_CEF_PATCH_CHECK=1 to override." >&2
        echo "   See docs/specs/SPEC_PATCHED_MACOS_CEF_FRAMEWORK_RELEASE_2026_06_29.md." >&2
        exit 1
    fi
fi

echo "==> Assembling AgentMux.app v$VERSION ($ARCH)"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"

cp dist/cef/agentmux-cef "$APP/Contents/MacOS/agentmux-cef"
cp "$SRV" "$APP/Contents/MacOS/$(basename "$SRV")"

# The LAUNCHER is the bundle entry point (CFBundleExecutable below): a tiny,
# fast binary that paints the native splash INSTANTLY — before the multi-second
# CEF host load — then spawns + supervises srv and the host (run_unix). The host
# stays a sibling under MacOS/, so its current_exe()-relative resolution of
# frontend/srv/../Frameworks and its Dock tile are byte-identical to the
# host-as-entry-point build. The launcher runs as an accessory (no Dock tile of
# its own); the host sets the regular policy and owns the one tile.
cp dist/cef/agentmux-launcher "$APP/Contents/MacOS/agentmux-launcher"

# Bundled tools — agentmux-srv adds <exe_dir>/tools/bin to Claude's PATH.
# On macOS, exe_dir = Contents/MacOS, so tools land at Contents/MacOS/tools/bin/.
mkdir -p "$APP/Contents/MacOS/tools/bin"
cp target/release/agentmux-mcp "$APP/Contents/MacOS/tools/bin/agentmux-mcp"

# Frontend is a tree of resource files (HTML/CSS/fonts), NOT code. codesign
# only allows executables under Contents/MacOS/ — a resource dir there breaks
# the bundle seal ("In subcomponent: …/frontend/…css"). So the real files live
# in Contents/Resources/frontend (sealed automatically as bundle resources),
# and a relative symlink at Contents/MacOS/frontend keeps the host's
# `current_exe().parent()/frontend` lookup working with no Rust change.
cp -R dist/frontend "$APP/Contents/Resources/frontend"
ln -s "../Resources/frontend" "$APP/Contents/MacOS/frontend"

# Strip .js.map source maps for release DMGs (~28 MB saved). Matches the
# STRIP_MAPS logic in scripts/package.sh (#1226). Set STRIP_MAPS=0 to keep.
if [ "${STRIP_MAPS:-1}" = "1" ]; then
    find "$APP/Contents/Resources/frontend" -name "*.map" -delete
    echo "  stripped .map files from frontend"
fi

# Schema files (JSON) — srv resolves these from AGENTMUX_APP_PATH/schema
# which is Contents/MacOS/ (the host exe's parent directory). Resource files
# cannot live under Contents/MacOS/ without breaking the bundle seal, so
# place them in Resources/schema and symlink like frontend.
cp -R dist/schema "$APP/Contents/Resources/schema"
ln -s "../Resources/schema" "$APP/Contents/MacOS/schema"

# GL libs next to the host exe (Chromium DIR_MODULE = exe dir). These are
# Mach-O dylibs, so codesign signs them as nested code — fine under MacOS/.
# NOTE: vk_swiftshader_icd.json (the SwiftShader Vulkan ICD manifest) is
# intentionally NOT copied: it's a non-code JSON file, which codesign refuses
# to seal under Contents/MacOS/, and it's only the software-Vulkan fallback —
# not needed with hardware GL (libGLESv2/libEGL). If a future build must ship
# it, place it in Resources/ with a library_path that points back at MacOS/.
shopt -s nullglob
for f in dist/cef/*.dylib; do cp "$f" "$APP/Contents/MacOS/"; done
shopt -u nullglob

# CEF framework — ditto preserves the Versions/Current symlink chain the loader follows.
ditto "dist/Frameworks/Chromium Embedded Framework.framework" \
      "$APP/Contents/Frameworks/Chromium Embedded Framework.framework"

# CEF Helper apps — on macOS, Chromium launches renderer/GPU/utility
# subprocesses as the standard per-type helper apps named "<App> Helper (<Type>)"
# (CEF's canonical macOS layout). Each is a copy of the host binary (which
# handles --type subprocess mode), with a distinct bundle id + LSUIElement, so
# the macOS process model accepts them instead of re-execing the main bundle.
# (The patched CEF framework additionally disables the Mach-port peer
# process_requirement validation that failed on macOS 26 — the deeper fix.)
# The patched from-source CEF 148 framework spawns renderer/GPU/utility
# subprocesses as per-type named helpers (Renderer/GPU/Plugin/Alloy) regardless
# of browser_subprocess_path — it derives the path from the bundle name directly.
# Five variants are required:
#   Generic  — fallback + resolve_browser_subprocess_path() target
#   Renderer / GPU / Plugin / Alloy — spawned by the patched CEF 148 framework
# Alerts (Chromium's native notification service) is intentionally excluded.
# AgentMux never uses OS notifications; including the helper triggers macOS to
# show a "Notifications may include alerts…" permission prompt on first launch
# of every new version (dual-bundle registration — main app + Alerts helper each
# prompt independently). With --disable-notifications in on_before_command_line_
# processing (agentmux-cef/src/app.rs) CEF never spawns the Alerts helper, so
# omitting it from the bundle is safe and eliminates both prompts permanently.
# See docs/retro/retro-macos-notification-double-prompt-regression-2026-06-22.md
HELPER_NAMES=("AgentMux Helper" "AgentMux Helper (GPU)" "AgentMux Helper (Plugin)" \
              "AgentMux Helper (Renderer)" "AgentMux Helper (Alloy)")
HELPER_IDS=("helper" "helper.gpu" "helper.plugin" "helper.renderer" "helper.alloy")
HELPER_APPS=()
shopt -s nullglob
for i in "${!HELPER_NAMES[@]}"; do
    hn="${HELPER_NAMES[$i]}"
    ha="$APP/Contents/Frameworks/${hn}.app"
    mkdir -p "$ha/Contents/MacOS"
    cp dist/cef/agentmux-cef "$ha/Contents/MacOS/${hn}"
    # GL libs next to generic + GPU helpers only — the GPU subprocess resolves
    # them via its DIR_MODULE; the generic helper needs them as a fallback.
    # Renderer/Plugin/Alloy never touch GL.
    case "$hn" in
        "AgentMux Helper"|"AgentMux Helper (GPU)")
            for f in dist/cef/*.dylib; do cp "$f" "$ha/Contents/MacOS/"; done ;;
    esac
    cat > "$ha/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>${hn}</string>
    <key>CFBundleDisplayName</key><string>${hn}</string>
    <key>CFBundleExecutable</key><string>${hn}</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID}.${HELPER_IDS[$i]}</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
    HELPER_APPS+=("$ha")
done
shopt -u nullglob

# Icon: build AgentMux.icns from the 512px PNG (the normal AgentMux logo, same
# source the Dock icon + Linux taskbar use).
echo "==> Generating AgentMux.icns"
SRC_PNG="assets/linux/icons/hicolor/512x512/apps/agentmux.png"
require "$SRC_PNG"
ICONSET="$(mktemp -d)/AgentMux.iconset"; mkdir -p "$ICONSET"
for s in 16 32 64 128 256 512; do
  sips -z "$s" "$s" "$SRC_PNG" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2)); sips -z "$d" "$d" "$SRC_PNG" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AgentMux.icns"
# Info.plist
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>AgentMux</string>
    <key>CFBundleDisplayName</key><string>AgentMux</string>
    <key>CFBundleExecutable</key><string>agentmux-launcher</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleIconFile</key><string>AgentMux</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <!-- Launcher is a UIElement: it shows the splash, then CEF takes over as the
         sole Foreground app (Dock tile, menu bar). Without this, macOS 26 Tahoe
         ignores the runtime setActivationPolicy(.accessory) call in splash_mac.rs
         and the launcher retains its own Dock slot alongside CEF's, producing two
         icons. LSUIElement=true prevents the OS from ever registering the launcher
         as Foreground, eliminating the duplicate. See BUG_MACOS26_DUAL_DOCK_ICON. -->
    <key>LSUIElement</key><true/>
    <!-- Opt-in capability prompts. These usage strings are shown ONLY if/when
         the user turns the feature on — neither resource is touched at launch:
         the microphone (a pane-header mic button, Web Speech API) and local
         network / mDNS (the "LAN discovery" switch in the status bar, default
         off). So neither prompts until the user enables it. -->
    <key>NSMicrophoneUsageDescription</key><string>AgentMux uses the microphone only when you turn on voice input from a pane's microphone button, to transcribe what you say into that pane.</string>
    <key>NSLocalNetworkUsageDescription</key><string>AgentMux uses your local network only when you turn on "LAN discovery" in the status bar, to find other AgentMux instances on your network.</string>
    <key>NSBonjourServices</key>
    <array>
        <string>_agentmux._tcp</string>
    </array>
</dict>
</plist>
PLIST

FW="$APP/Contents/Frameworks/Chromium Embedded Framework.framework"

# Strip debug/local symbols from every Mach-O BEFORE signing (a from-source
# CEF build with symbol_level=1 leaves ~240MB of symbols in libcef alone).
# `strip -S -x` removes debug info + local symbols while keeping the exported
# (global) symbols that cef-rs's loader needs. Halves the framework; the GL
# dylibs and the host/helper copies shrink too. Must run before codesign (it
# would otherwise invalidate the signature).
echo "==> Stripping symbols (lean build)"
strip -S -x "$FW/Versions/A/Chromium Embedded Framework" 2>/dev/null || true
shopt -s nullglob
for dy in "$FW/Versions/A/Libraries/"*.dylib \
          "$APP/Contents/MacOS/"*.dylib \
          "$APP/Contents/MacOS/agentmux-cef" \
          "$APP/Contents/MacOS/agentmux-launcher" \
          "$APP/Contents/Frameworks/"*Helper*.app/Contents/MacOS/*.dylib \
          "$APP/Contents/Frameworks/"*Helper*.app/Contents/MacOS/"AgentMux Helper"*; do
    strip -S -x "$dy" 2>/dev/null || true
done
shopt -u nullglob
# Trim non-English locales (~52MB of locale.pak across ~200 languages). These
# are only Chromium's built-in UI strings (context menus, etc.) — AgentMux's UI
# is its own bundled frontend. Chromium falls back to en for any locale whose
# .pak is absent, so keeping en* is safe and standard for size-conscious builds.
LOCDIR="$FW/Versions/A/Resources"
for lp in "$LOCDIR"/*.lproj; do
    case "$(basename "$lp")" in
        en.lproj|en_*.lproj|en-*.lproj) ;;
        *) rm -rf "$lp" ;;
    esac
done
echo "  framework now: $(du -sh "$FW" 2>/dev/null | awk '{print $1}')  (locales trimmed to en*)"

echo "==> Signing inside-out (Developer ID: $CERT)"
SIGN=(codesign --force --timestamp --options runtime --sign "$CERT")

# 1. GL dylibs next to the exe (plain libraries — no entitlements).
shopt -s nullglob
for dy in "$APP/Contents/MacOS/"*.dylib; do "${SIGN[@]}" "$dy"; done
# 2. Framework's nested dylibs, then the framework bundle itself.
for dy in "$FW/Libraries/"*.dylib; do "${SIGN[@]}" "$dy"; done
# 3. Each Helper app: its GL dylibs → helper exe (entitlements: the renderer's
#    V8 JIT + framework dlopen) → helper bundle. Signed before the outer .app so
#    the signatures are included in the bundle seal.
for ha in "${HELPER_APPS[@]}"; do
    for dy in "$ha/Contents/MacOS/"*.dylib; do "${SIGN[@]}" "$dy"; done
done
shopt -u nullglob
"${SIGN[@]}" "$FW"
for ha in "${HELPER_APPS[@]}"; do
    "${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$ha/Contents/MacOS/$(basename "$ha" .app)"
    "${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$ha"
done
# 4. Backend + host get the app entitlements (CLI feature access + CEF JIT).
#    The host is now NESTED code (the launcher is CFBundleExecutable), so it must
#    be signed here, before the bundle seal.
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$APP/Contents/MacOS/$(basename "$SRV")"
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$APP/Contents/MacOS/agentmux-cef"
# agentmux-mcp is a nested Mach-O under MacOS/tools/bin/ (Claude's PATH). It must
# be signed inside-out before the seal or `codesign --verify --deep --strict`
# fails on the unsigned binary and hardened-runtime/notarization rejects it.
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$APP/Contents/MacOS/tools/bin/agentmux-mcp"
# 5. Seal the .app bundle last. codesign signs the main executable
#    (agentmux-launcher) as part of sealing; pass the entitlements so the
#    launcher is hardened-runtime signed identically to the host.
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$APP"

echo "==> Verifying signature"
codesign --verify --deep --strict --verbose=2 "$APP"

echo "==> Creating DMG (icon-view drag-to-install layout)"
mkdir -p "$OUTDIR"
rm -f "$DMG"
DMG_VOL="AgentMux"
STAGE="$(mktemp -d)/dmg"; mkdir -p "$STAGE"
ditto "$APP" "$STAGE/AgentMux.app"
ln -s /Applications "$STAGE/Applications"
# Build a READ-WRITE DMG first so we can set the Finder window (icon view, the
# AgentMux app icon on the left + an /Applications drop-target on the right —
# the standard drag-to-install UX), then compress it read-only. Without this a
# plain DMG opens in whatever view Finder last used (list/column), showing the
# bundle as folders. Falls back to a plain DMG if Finder automation is
# unavailable (headless / no TCC grant).
RW="$(mktemp -u).dmg"
# Use APFS (not HFS+) for the intermediate read-write volume. macOS 26 Tahoe
# has a regression in HFS+ transparent file decompression (decmpfs, the `z`
# attribute that hdiutil applies automatically to compressible files like JS/HTML
# when building an HFS+ image). On the mounted HFS+ DMG, reading those files
# fails with ENOTTY / Input/output error — the IPC server sets Content-Length
# from file metadata but then can't stream the body, causing ERR_CONTENT_LENGTH_MISMATCH
# in CEF. APFS images get the same `z` attribute but their decompressor is
# unaffected on macOS 26, so switching here fixes the bug. APFS DMGs have been
# mountable since macOS 10.14 and we target 11+, so this is safe.
hdiutil create -volname "$DMG_VOL" -srcfolder "$STAGE" -ov -format UDRW -fs APFS "$RW" >/dev/null
RWMNT="$(hdiutil attach "$RW" -nobrowse -noautoopen 2>/dev/null | sed -n 's/.*\(\/Volumes\/.*\)/\1/p' | tail -1)"
if [ -n "$RWMNT" ]; then
    osascript >/dev/null 2>&1 <<OSA || echo "  ⚠ Finder layout skipped (automation unavailable) — DMG uses default view"
tell application "Finder"
    tell disk "$DMG_VOL"
        open
        set current view of container window to icon view
        set toolbar visible of container window to false
        set statusbar visible of container window to false
        set the bounds of container window to {300, 150, 880, 520}
        set vopts to the icon view options of container window
        set arrangement of vopts to not arranged
        set icon size of vopts to 128
        set position of item "AgentMux.app" of container window to {150, 195}
        set position of item "Applications" of container window to {430, 195}
        update without registering applications
        delay 1
        close
    end tell
end tell
OSA
    sync
    hdiutil detach "$RWMNT" >/dev/null 2>&1 || true
fi
# ULMO (LZMA) compression — LZMA-class like Linux's SquashFS AppImage, vs the
# default UDZO (zlib) which left the DMG ~50% larger. Requires macOS 10.15+ to
# mount (we target 11+, so fine). On this build: UDZO 248MB -> ULMO 167MB.
# Purge inactive pages first — LZMA peaks at 4-6GB RAM and gets OOM-killed if
# the system is under memory pressure after the Rust compile phase.
sudo -n purge 2>/dev/null || true
hdiutil convert "$RW" -format ULMO -o "$DMG" >/dev/null
rm -f "$RW"
codesign --force --timestamp --sign "$CERT" "$DMG"

NOTARIZED=0
if [ "$NOTARIZE" = "1" ]; then
    echo "==> Notarizing (keychain profile: $NOTARY_PROFILE)"
    # Capture the full output FIRST, then inspect — do NOT pipe into `grep -q`.
    # `grep -q` exits on the first match (the mid-stream "Current status:
    # Accepted"), which SIGPIPEs notarytool; under `set -o pipefail` that makes
    # the whole pipeline non-zero and the success branch is skipped even though
    # Apple accepted the submission (notarized-but-not-stapled). `|| true` keeps
    # set -e from aborting if notarytool exits non-zero.
    notary_out="$(xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait 2>&1 || true)"
    printf '%s\n' "$notary_out" > /tmp/agentmux-notary.log
    if printf '%s\n' "$notary_out" | grep -q "status: Accepted"; then
        xcrun stapler staple "$DMG"
        NOTARIZED=1
        echo "✓ Notarized + stapled"
    else
        echo "⚠ Notarization not accepted — emitting a SIGNED-ONLY DMG. Log: /tmp/agentmux-notary.log"
        echo "  If this is an agreement issue, re-sign at https://appstoreconnect.apple.com"
        echo "  (notarytool returns HTTP 403 'a required agreement is missing or has expired')."
    fi
else
    echo "==> Skipping notarization (NOTARIZE=0)"
fi

echo ""
if [ "$NOTARIZED" = "1" ]; then
    echo "✓ Built SIGNED + NOTARIZED DMG: $DMG"
else
    echo "✓ Built SIGNED (not notarized) DMG: $DMG"
fi
codesign -dv --verbose=2 "$DMG" 2>&1 | grep -E "Authority=|TeamIdentifier=" | head -3 || true
ls -lh "$DMG"
