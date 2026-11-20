#!/usr/bin/env bash
# Notarize a signed AgentMux.app or .dmg with Apple, staple the ticket to it and check the staple.
#
# Usage:  notarize-macos.sh app <AgentMux.app>
#         notarize-macos.sh dmg <AgentMux.dmg>
#
# An app goes to Apple as a zip (the zip is only the vehicle; the ticket is stapled to the .app itself), a disk
# image goes as it is. The order a release uses (build-macos.yml, and package-macos.sh when NOTARIZE_APP=1):
#   sign the app → notarize and staple the app → make the DMG from the stapled app → sign the DMG → notarize and
#   staple the DMG. The app inside the DMG carries its own ticket, so a first launch needs no network for
#   Gatekeeper's online check, and the DMG carries one for the download itself.
# The ticket for a stapled file is written into the file, so staple BEFORE anything hashes or publishes it.
#
# NOTARY_PROFILE   keychain profile name (default: notarytool)
# KEYCHAIN_PATH    the keychain that holds the profile, when it is not in the default list
# NOTARY_LOG       where the full notarytool output is kept (default: a temp file, path printed)
#
# Exit: 0 accepted, stapled and `stapler validate` passes;
#       2 Apple did not accept it (the log says why: an expired agreement is HTTP 403) — nothing stapled;
#       1 anything else (bad usage, a staple or a validation that fails).
set -euo pipefail

KIND="${1:-}"
TARGET="${2:-}"
NOTARY_PROFILE="${NOTARY_PROFILE:-notarytool}"
LOG="${NOTARY_LOG:-$(mktemp -t agentmux-notary).log}"

case "$KIND" in
    app) [ -d "$TARGET/Contents" ] || { echo "❌ notarize: not an .app: ${TARGET:-<none>}" >&2; exit 1; } ;;
    dmg) [ -f "$TARGET" ] || { echo "❌ notarize: not a file: ${TARGET:-<none>}" >&2; exit 1; } ;;
    *) echo "usage: notarize-macos.sh app|dmg <path>" >&2; exit 1 ;;
esac

SUBMIT="$TARGET"
ZIPDIR=""
if [ "$KIND" = "app" ]; then
    ZIPDIR="$(mktemp -d)"
    trap 'rm -rf "$ZIPDIR"' EXIT
    SUBMIT="$ZIPDIR/$(basename "$TARGET" .app).zip"
    ditto -c -k --keepParent "$TARGET" "$SUBMIT"
fi

echo "==> Notarizing the $KIND (keychain profile: $NOTARY_PROFILE)"
# Capture the full output FIRST, then inspect — do NOT pipe into `grep -q`.
# `grep -q` exits on the first match (the mid-stream "Current status:
# Accepted"), which SIGPIPEs notarytool; under `set -o pipefail` that makes
# the whole pipeline non-zero and the success branch is skipped even though
# Apple accepted the submission (notarized-but-not-stapled). `|| true` keeps
# set -e from aborting if notarytool exits non-zero.
out="$(xcrun notarytool submit "$SUBMIT" --keychain-profile "$NOTARY_PROFILE" ${KEYCHAIN_PATH:+--keychain "$KEYCHAIN_PATH"} --wait 2>&1 || true)"
printf '%s\n' "$out" | tee "$LOG"
if ! printf '%s\n' "$out" | grep -q "status: Accepted"; then
    echo "⚠ Apple did not accept the $KIND. Log: $LOG" >&2
    echo "  If this is an agreement issue, re-sign at https://appstoreconnect.apple.com" >&2
    echo "  (notarytool returns HTTP 403 'a required agreement is missing or has expired')." >&2
    exit 2
fi

xcrun stapler staple "$TARGET"
# `staple` can succeed on a file whose ticket then does not validate (the ticket is looked up by the file's hash);
# the release stops here, not at a user's first launch.
xcrun stapler validate "$TARGET"
echo "✓ Notarized, stapled and validated: $TARGET"
