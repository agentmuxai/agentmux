#!/usr/bin/env bash
# Fail unless every srv binary in a macOS AgentMux.app is signed with the fixed
# identity the login keychain trusts.
#
# Why
# ---
# srv is the process that reads the macOS keychain, and a keychain item trusts
# an app by its designated requirement (DR): the code identifier plus the
# signing team. srv has been signed as `ai.agentmux.srv` since v0.59.16 (#4482),
# which is what keeps a new version from asking for the keychain again. That
# fix was lost once before, and a release whose srv comes out with another
# identifier, or with another team, makes every user's every keychain item ask
# again on first launch. package-macos.sh runs this after signing, before the
# DMG and notarization, so such a build never reaches either.
#
# Checked: `Contents/Helpers/AgentMux Server.app` (required) and every
# `Contents/MacOS/agentmux-srv*` (symlinks into it today). Each one's DR must
# start `identifier "ai.agentmux.srv" and anchor apple generic` and name the
# signing certificate's team (`certificate leaf[subject.OU] = "<team>"`).
#
# Usage:  verify-srv-identity-darwin.sh <AgentMux.app> <signing cert name | sha1>
#   The team is read from that certificate in the keychain; never written down
#   here. EXPECTED_TEAM_ID overrides it (tests); CODESIGN names the codesign to
#   run (tests).
#
# Exit:   0  every srv has the fixed identifier and the signing team
#         1  one doesn't, none was found, or the team couldn't be read

set -uo pipefail

APP="${1:-}"
CERT="${2:-}"
CODESIGN="${CODESIGN:-codesign}"
IDENTIFIER="ai.agentmux.srv"

[ -d "$APP" ] || { echo "❌ srv identity check: not a directory: ${APP:-<none>}" >&2; exit 1; }

# The signing certificate's team: the OU of its subject.
cert_team() {
    local pem
    if [[ "$CERT" =~ ^[0-9A-Fa-f]{40}$ ]]; then
        pem="$(security find-certificate -a -Z -p 2>/dev/null | awk -v h="$(printf '%s' "$CERT" | tr a-f A-F)" '
            /^SHA-1 hash:/ { on = ($3 == h); next }
            on' )"
    else
        pem="$(security find-certificate -c "$CERT" -p 2>/dev/null)"
    fi
    printf '%s\n' "$pem" | openssl x509 -noout -subject -nameopt RFC2253 2>/dev/null \
        | tr ',' '\n' | sed -n 's/^OU=//p' | head -1
}

TEAM="${EXPECTED_TEAM_ID:-}"
[ -n "$TEAM" ] || TEAM="$(cert_team)"
if ! [[ "$TEAM" =~ ^[A-Z0-9]{10}$ ]]; then
    echo "❌ srv identity check: can't read the signing certificate's team from the keychain (cert: ${CERT:-<none>})." >&2
    exit 1
fi

TARGETS=("$APP/Contents/Helpers/AgentMux Server.app")
if [ ! -d "${TARGETS[0]}" ]; then
    echo "❌ srv identity check: ${TARGETS[0]} is missing; srv is no longer where this check looks." >&2
    exit 1
fi
shopt -s nullglob
for f in "$APP"/Contents/MacOS/agentmux-srv*; do TARGETS+=("$f"); done
shopt -u nullglob

status=0
for t in "${TARGETS[@]}"; do
    dr="$("$CODESIGN" -d -r- "$t" 2>&1 | sed -n 's/^designated => //p')"
    found="$(printf '%s' "$dr" | sed -n 's/^identifier "\([^"]*\)".*/\1/p')"
    if [[ "$dr" == "identifier \"$IDENTIFIER\" and anchor apple generic"* ]] \
        && [[ "$dr" == *"certificate leaf[subject.OU] = \"$TEAM\""* ]]; then
        echo "  ✓ $t: $IDENTIFIER"
        continue
    fi
    echo "❌ srv identity check: $t" >&2
    echo "   identifier found: ${found:-<none>} (must be $IDENTIFIER, signed by this build's Developer ID team; a requirement that names another team or no certificate fails too)" >&2
    echo "   designated requirement: ${dr:-<none>}" >&2
    status=1
done
if [ "$status" -ne 0 ]; then
    echo "   A different srv identity makes every macOS keychain item ask again after the upgrade." >&2
    echo "   srv is signed with --identifier $IDENTIFIER in scripts/package-macos.sh; check that step." >&2
fi
exit "$status"
