#!/usr/bin/env bash
# Sign an assembled, unsigned AgentMux.app inside-out (Developer ID, hardened runtime), then verify it.
#
# Usage:  sign-macos-app.sh <AgentMux.app> <entitlements.plist> [signing certificate name]
#
# This is the signing half of what scripts/package-macos.sh used to do in one piece. The release workflow
# (build-macos.yml) runs it in the one job that holds the Developer ID certificate, taken from the revision of
# the workflow file itself, never from the commit that was built; a local build runs it from package-macos.sh.
# It only reads the .app (codesign, verification): it never starts a program from it.
#
# The certificate comes from the argument, else MACOS_SIGN_CERT, else the first 'Developer ID Application'
# identity in the keychain (KEYCHAIN_PATH names one, else the default list). The identity (name and team id) is
# never written down in this public repository.
# CODESIGN_TIMESTAMP overrides the secure timestamp flag (tests with an ad hoc identity use --timestamp=none).
#
# What is signed, in order (the same order package-macos.sh has always used; nested code before what holds it):
#   1. the GL dylibs next to the host exe
#   2. the framework's nested dylibs, then the framework
#   3. each CEF helper app (Contents/Frameworks/*Helper*.app): its dylibs, then its exe and bundle with entitlements
#   4. srv (fixed identifier ai.agentmux.srv), mcp, bashwrap, their tool apps, the host exe, the remote helpers
#   5. the .app itself, last
# The names are not hard-coded one by one: the helper apps and the tool apps are what the two directories hold, so
# a new helper is signed too. A layout this script does not know fails the verification instead of shipping unsigned.
#
# Exit: 0 signed and verified; 1 anything else.
set -euo pipefail

APP="${1:-}"
ENTITLEMENTS="${2:-}"
CERT="${3:-${MACOS_SIGN_CERT:-}}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

[ -d "$APP/Contents" ] || { echo "❌ sign: not an .app: ${APP:-<none>}" >&2; exit 1; }
[ -f "$ENTITLEMENTS" ] || { echo "❌ sign: missing entitlements: ${ENTITLEMENTS:-<none>}" >&2; exit 1; }

if [ -z "$CERT" ]; then
    CERT="$(security find-identity -v -p codesigning ${KEYCHAIN_PATH:+"$KEYCHAIN_PATH"} 2>/dev/null \
        | awk -F'"' '/Developer ID Application/ {print $2; exit}')"
fi
[ -n "$CERT" ] || { echo "❌ No 'Developer ID Application' identity in the keychain (and MACOS_SIGN_CERT unset)." >&2; exit 1; }

FW="$APP/Contents/Frameworks/Chromium Embedded Framework.framework"
[ -d "$FW" ] || { echo "❌ sign: the CEF framework is missing from the app" >&2; exit 1; }

shopt -s nullglob
HELPER_APPS=("$APP"/Contents/Frameworks/*Helper*.app)
TOOL_APPS=("$APP"/Contents/Helpers/*.app)
shopt -u nullglob
[ "${#HELPER_APPS[@]}" -gt 0 ] || { echo "❌ sign: no CEF helper apps in the app" >&2; exit 1; }
SRV_APP_BIN="$APP/Contents/Helpers/AgentMux Server.app/Contents/MacOS"
MCP_APP_BIN="$APP/Contents/Helpers/AgentMux MCP.app/Contents/MacOS"
BASHWRAP_APP_BIN="$APP/Contents/Helpers/AgentMux Shell Wrapper.app/Contents/MacOS"
for f in "$SRV_APP_BIN/agentmux-srv" "$MCP_APP_BIN/agentmux-mcp" "$BASHWRAP_APP_BIN/agentmux-bashwrap" "$APP/Contents/MacOS/agentmux-cef"; do
    [ -f "$f" ] || { echo "❌ sign: missing from the app: $f" >&2; exit 1; }
done

echo "==> Signing inside-out (Developer ID: $CERT)"
SIGN=(codesign --force "${CODESIGN_TIMESTAMP:---timestamp}" --options runtime --sign "$CERT" ${KEYCHAIN_PATH:+--keychain "$KEYCHAIN_PATH"})

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
#    The host is NESTED code (the launcher is CFBundleExecutable), so it must
#    be signed here, before the bundle seal.
#    srv gets a fixed --identifier: without one, codesign takes it from the
#    versioned file name, and srv is the process that reads the Keychain. An
#    item trusts an app by its designated requirement, identifier included, so
#    "Always Allow" never carried over and every new version asked for the
#    login keychain on first launch (retro-keychain-prompt-recurs-per-build-
#    identity-2026-08-21.md). Same identifier on every channel and version.
#    srv lives in its own helper app, whose CFBundleIdentifier is that same
#    ai.agentmux.srv. Signing the bundle below re-signs its main executable
#    with the bundle's identifier, so that plist entry is what keeps the
#    identifier; the explicit --identifier here only matches it.
#    agentmux-mcp and agentmux-bashwrap (on Claude's PATH through the
#    MacOS/tools/bin symlinks) are signed the same way, inside-out, before the
#    seal, or `codesign --verify --deep --strict` fails and notarization
#    rejects the bundle. The symlinks themselves are sealed as resources.
"${SIGN[@]}" --identifier ai.agentmux.srv --entitlements "$ENTITLEMENTS" "$SRV_APP_BIN/agentmux-srv"
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$MCP_APP_BIN/agentmux-mcp"
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$BASHWRAP_APP_BIN/agentmux-bashwrap"
for ta in "${TOOL_APPS[@]}"; do
    "${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$ta"
done
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$APP/Contents/MacOS/agentmux-cef"
# The two macOS remote helpers are nested Mach-Os as well, signed the same way
# (no entitlements: they run on a remote Mac, not in this app). The Linux builds
# under Contents/Resources/remote are not code and are sealed as resources.
for helper in "$APP"/Contents/MacOS/tools/remote/*/agentmux-remote; do
    [ -f "$helper" ] && "${SIGN[@]}" "$helper"
done
# 5. Seal the .app bundle last. codesign signs the main executable
#    (agentmux-launcher) as part of sealing; pass the entitlements so the
#    launcher is hardened-runtime signed identically to the host.
"${SIGN[@]}" --entitlements "$ENTITLEMENTS" "$APP"

echo "==> Verifying signature"
codesign --verify --deep --strict --verbose=2 "$APP"

# srv must still be the app the login keychain trusts: identifier
# ai.agentmux.srv, signed by this cert's team. A build that comes out
# otherwise makes every keychain item ask again after the upgrade, so it stops
# here, before the DMG and notarization (it lost this once before, #4482).
echo "==> Verifying srv's signing identity"
bash "$SCRIPT_DIR/verify-srv-identity-darwin.sh" "$APP" "$CERT" \
    || { echo "❌ srv is not signed as ai.agentmux.srv by the signing team; not packaging." >&2; exit 1; }

# The stable app's signed requirement must name the fixed id, with no version: that is what lets macOS keep its
# privacy grants across upgrades (the same check ran on the unsigned app; this one reads the signature).
echo "==> Verifying the app's bundle id and requirement"
bash "$SCRIPT_DIR/verify-app-identity-darwin.sh" "$APP" --signed \
    || { echo "❌ the app's bundle id or signed requirement is not the fixed one; not packaging." >&2; exit 1; }
