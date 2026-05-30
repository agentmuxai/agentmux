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
#       agentmux-cef                          ← host (CFBundleExecutable; re-execs
#                                                itself for renderer/gpu subprocesses)
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
BUNDLE_ID="ai.agentmux.cef"
NOTARIZE="${NOTARIZE:-1}"
NOTARY_PROFILE="${NOTARY_PROFILE:-notarytool}"
SRV="dist/bin/agentmux-srv-${VERSION}-darwin.${ARCH}"

require() { [ -e "$1" ] || { echo "❌ missing required artifact: $1 — run the build steps first" >&2; exit 1; }; }
require dist/cef/agentmux-cef
require "$SRV"
require dist/frontend/index.html
require "dist/Frameworks/Chromium Embedded Framework.framework"
[ -f "$ENTITLEMENTS" ] || { echo "❌ missing entitlements: $ENTITLEMENTS" >&2; exit 1; }

echo "==> Assembling AgentMux.app v$VERSION ($ARCH)"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"

cp dist/cef/agentmux-cef "$APP/Contents/MacOS/agentmux-cef"
cp "$SRV" "$APP/Contents/MacOS/$(basename "$SRV")"

# Frontend is a tree of resource files (HTML/CSS/fonts), NOT code. codesign
# only allows executables under Contents/MacOS/ — a resource dir there breaks
# the bundle seal ("In subcomponent: …/frontend/…css"). So the real files live
# in Contents/Resources/frontend (sealed automatically as bundle resources),
# and a relative symlink at Contents/MacOS/frontend keeps the host's
# `current_exe().parent()/frontend` lookup working with no Rust change.
cp -R dist/frontend "$APP/Contents/Resources/frontend"
ln -s "../Resources/frontend" "$APP/Contents/MacOS/frontend"

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

# CEF Helper app — renderer/GPU/utility subprocesses run as this dedicated
# helper (distinct bundle id, LSUIElement) instead of re-execing the main
# bundle binary, which the macOS process model rejects (→ renderer crash-loop).
# The host points CEF's browser_subprocess_path at it (see
# main.rs::resolve_browser_subprocess_path). One helper suffices because all
# subprocess types share one entitlement set.
HELPER_APP="$APP/Contents/Frameworks/AgentMux Helper.app"
mkdir -p "$HELPER_APP/Contents/MacOS"
cp dist/cef/agentmux-cef "$HELPER_APP/Contents/MacOS/AgentMux Helper"
# GL libs next to the helper exe too — the GPU subprocess resolves them via its
# own DIR_MODULE (= the helper's MacOS dir).
shopt -s nullglob
for f in dist/cef/*.dylib; do cp "$f" "$HELPER_APP/Contents/MacOS/"; done
shopt -u nullglob
cat > "$HELPER_APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>AgentMux Helper</string>
    <key>CFBundleDisplayName</key><string>AgentMux Helper</string>
    <key>CFBundleExecutable</key><string>AgentMux Helper</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID}.helper</string>
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
    <key>CFBundleExecutable</key><string>agentmux-cef</string>
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
</dict>
</plist>
PLIST

echo "==> Signing inside-out (Developer ID: $CERT)"
SIGN=(codesign --force --timestamp --options runtime --sign "$CERT")
FW="$APP/Contents/Frameworks/Chromium Embedded Framework.framework"

# 1. GL dylibs next to the exe (plain libraries — no entitlements).
shopt -s nullglob
for dy in "$APP/Contents/MacOS/"*.dylib; do "${SIGN[@]}" "$dy"; done
# 2. Framework's nested dylibs, then the framework bundle itself.
for dy in "$FW/Libraries/"*.dylib; do "${SIGN[@]}" "$dy"; done
# 3. Helper app: its GL dylibs → helper exe (entitlements: the renderer's V8
#    JIT + framework dlopen) → helper bundle. Signed before the outer .app so
#    its signature is included in the bundle seal.
for dy in "$HELPER_APP/Contents/MacOS/"*.dylib; do "${SIGN[@]}" "$dy"; done
shopt -u nullglob
"${SIGN[@]}" "$FW"
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$HELPER_APP/Contents/MacOS/AgentMux Helper"
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$HELPER_APP"
# 4. Backend + host get the app entitlements (CLI feature access + CEF JIT).
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$APP/Contents/MacOS/$(basename "$SRV")"
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$APP/Contents/MacOS/agentmux-cef"
# 5. Seal the .app bundle last (everything nested is already signed).
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$APP"

echo "==> Verifying signature"
codesign --verify --deep --strict --verbose=2 "$APP"

echo "==> Creating DMG"
mkdir -p "$OUTDIR"
rm -f "$DMG"
STAGE="$(mktemp -d)/dmg"; mkdir -p "$STAGE"
ditto "$APP" "$STAGE/AgentMux.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "AgentMux ${VERSION}" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
codesign --force --timestamp --sign "$CERT" "$DMG"

NOTARIZED=0
if [ "$NOTARIZE" = "1" ]; then
    echo "==> Notarizing (keychain profile: $NOTARY_PROFILE)"
    if xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait 2>&1 \
         | tee /tmp/agentmux-notary.log | grep -q "status: Accepted"; then
        xcrun stapler staple "$DMG"
        NOTARIZED=1
        echo "✓ Notarized + stapled"
    else
        echo "⚠ Notarization FAILED or unavailable — emitting a SIGNED-ONLY DMG."
        echo "  Log: /tmp/agentmux-notary.log"
        echo "  Most likely the Apple Developer program license agreement needs"
        echo "  re-signing at https://appstoreconnect.apple.com (notarytool returns"
        echo "  HTTP 403 'a required agreement is missing or has expired')."
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
