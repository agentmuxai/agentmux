#!/usr/bin/env bash
# Make the drag-to-install disk image from an AgentMux.app (unsigned image; signing and notarization are separate).
#
# Usage:  make-macos-dmg.sh <AgentMux.app> <output.dmg>
#
# The .app goes in as it is: signed and, for a release, already stapled (see scripts/notarize-macos.sh), so the
# ticket stapled to the app is inside the image. The image itself is signed and notarized by the caller after
# this, and the ticket for it is stapled last.
set -euo pipefail

APP="${1:-}"
DMG="${2:-}"
[ -d "$APP/Contents" ] || { echo "❌ dmg: not an .app: ${APP:-<none>}" >&2; exit 1; }
[ -n "$DMG" ] || { echo "usage: make-macos-dmg.sh <AgentMux.app> <output.dmg>" >&2; exit 1; }

echo "==> Creating DMG (icon-view drag-to-install layout)"
mkdir -p "$(dirname "$DMG")"
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
# ULFO (LZFSE) compression. Every page of the app read from a mounted DMG is
# decompressed on the way in: the first launch from a fresh mount, and the
# copy into /Applications. ULMO (LZMA) made that slow: on 0.59.20 (M2), reading
# the 309 MB Chromium framework from a fresh mount took 4.2 s with ULMO and
# 0.4 s with ULFO, and a cold launch from the mount 16.1 s vs 10.1 s. The cost
# is size: ULMO 170 MB, ULFO 223 MB (UDZO/zlib was ~248 MB). ULFO mounts on
# macOS 10.11+ (we target 11+).
hdiutil convert "$RW" -format ULFO -o "$DMG" >/dev/null
rm -f "$RW"
rm -rf "$(dirname "$STAGE")"
echo "✓ DMG: $DMG"
