#!/usr/bin/env bash
# Fail unless a stable-channel AgentMux.app carries the fixed bundle id, with no version in it.
#
# Why
# ---
# macOS keys its privacy grants (Screen Recording, Accessibility, folders, notifications) and the Login Items
# registration to an app's code signature requirement, and that requirement names the bundle id. While the id carried
# the version every release was a new app to macOS: each upgrade dropped the grants and left a stale row in System
# Settings. The stable channel is `ai.agentmux.stable` for every release, so a Developer-ID-signed upgrade keeps them
# (scripts/package-macos.sh, AGENTMUX_STABLE_BUNDLE_ID=1, set by the release's build job). A build that comes out
# with a version in the id, or whose signed requirement names another id, puts every person back to granting again, so
# it stops here: package-macos.sh runs this on the assembled app, and sign-macos-app.sh runs it again after signing
# (--signed), before the DMG and notarization.
#
# Checked, for an app whose id starts with `ai.agentmux.stable`:
#   - the Info.plist id is exactly `ai.agentmux.stable` (no `.0.60.36`, no other suffix);
#   - the Login Items agent plist `Contents/Library/LaunchAgents/ai.agentmux.stable.login.plist` is in the bundle;
#   - with --signed: the designated requirement is `identifier "ai.agentmux.stable" and anchor apple generic ...`.
# Another channel's app (`ai.agentmux.<channel>.<version>`, a local build) is not touched: it keeps its own id.
#
# The release's own build and signing jobs say what they expect: AGENTMUX_EXPECT_BUNDLE_ID=ai.agentmux.stable (set in
# .github/workflows/build-macos.yml, as a literal, not derived from the channel variable). With it set the app's id must
# be exactly that, whatever it starts with, so a build that lost its channel variable and came out as
# `ai.agentmux.<other>.<version>` fails here instead of passing as "another channel's app". Local builds leave it unset.
#
# Usage:  verify-app-identity-darwin.sh <AgentMux.app> [--signed]
#   CODESIGN names the codesign to run (tests).
#   AGENTMUX_EXPECT_BUNDLE_ID the id the app must have, exactly (the release jobs).
#
# Exit:   0  the app passes (or is not a stable-channel app)
#         1  it does not, or the app or its id could not be read

set -uo pipefail

APP="${1:-}"
MODE="${2:-}"
CODESIGN="${CODESIGN:-codesign}"
STABLE_ID="ai.agentmux.stable"
EXPECT="${AGENTMUX_EXPECT_BUNDLE_ID:-}"

[ -d "$APP" ] || { echo "❌ app identity check: not a directory: ${APP:-<none>}" >&2; exit 1; }
PLIST="$APP/Contents/Info.plist"
[ -f "$PLIST" ] || { echo "❌ app identity check: no Info.plist in $APP" >&2; exit 1; }

# CFBundleIdentifier from the XML plist (the key and its string may be on one line or two).
id="$(tr '\n' ' ' < "$PLIST" | sed -n 's/.*<key>CFBundleIdentifier<\/key>[[:space:]]*<string>\([^<]*\)<\/string>.*/\1/p')"
[ -n "$id" ] || { echo "❌ app identity check: no CFBundleIdentifier in $PLIST" >&2; exit 1; }

if [ -n "$EXPECT" ] && [ "$id" != "$EXPECT" ]; then
    echo "❌ app identity check: the app's bundle id is '$id', but this build must produce '$EXPECT'." >&2
    echo "   The release build sets the channel (RELEASE_CHANNEL=stable) and AGENTMUX_STABLE_BUNDLE_ID=1; one of them was lost." >&2
    exit 1
fi

case "$id" in
    "$STABLE_ID"|"$STABLE_ID".*) ;;
    *) echo "  ✓ $id is not a stable-channel id: left as it is"; exit 0 ;;
esac

status=0
if [ "$id" != "$STABLE_ID" ]; then
    echo "❌ app identity check: the stable app's bundle id is '$id', not '$STABLE_ID'." >&2
    echo "   A version (or any suffix) in the id makes every release a new app to macOS: each upgrade would drop" >&2
    echo "   Screen Recording, Accessibility and the other grants. Package with AGENTMUX_STABLE_BUNDLE_ID=1 (scripts/package-macos.sh)." >&2
    status=1
else
    echo "  ✓ bundle id: $id"
fi

agent="$APP/Contents/Library/LaunchAgents/$id.login.plist"
if [ "$status" -eq 0 ] && [ ! -f "$agent" ]; then
    echo "❌ app identity check: $agent is missing: start at login would fall back to a LaunchAgent file." >&2
    status=1
fi

if [ "$MODE" = "--signed" ] && [ "$status" -eq 0 ]; then
    dr="$("$CODESIGN" -d -r- "$APP" 2>&1 | sed -n 's/^designated => //p')"
    if [[ "$dr" == "identifier \"$id\" and anchor apple generic"* ]]; then
        echo "  ✓ designated requirement names $id"
    else
        found="$(printf '%s' "$dr" | sed -n 's/^identifier "\([^"]*\)".*/\1/p')"
        echo "❌ app identity check: the signed requirement names '${found:-<none>}' (must be $id, signed by a Developer ID)." >&2
        echo "   designated requirement: ${dr:-<none>}" >&2
        status=1
    fi
fi
exit "$status"
