#!/usr/bin/env bash
# verify-srv-identity-darwin.test.sh — tests for scripts/verify-srv-identity-darwin.sh.
#
# The script reads each srv's designated requirement with `codesign -d -r-`;
# these tests put a fake codesign (CODESIGN) in front of a fake bundle that
# prints a requirement per file, so they run anywhere. The last case signs a
# real copy ad hoc under the right identifier on macOS: it must be refused.
#
# Usage: bash scripts/verify-srv-identity-darwin.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/verify-srv-identity-darwin.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

TEAM="ABCDE12345"
GOOD="identifier \"ai.agentmux.srv\" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate leaf[subject.OU] = \"$TEAM\""

# A fake codesign: prints "designated => <contents of $FAKE_DR/<basename>>" for
# the last argument, like `codesign -d -r- <path>` (the Executable= line goes
# to stderr, which the script merges).
mkdir -p "$TMP/bin"
cat > "$TMP/bin/codesign" <<'FAKE'
#!/usr/bin/env bash
for last; do :; done
echo "Executable=$last" >&2
f="$FAKE_DR/$(basename "$last")"
[ -f "$f" ] && echo "designated => $(cat "$f")"
exit 0
FAKE
chmod +x "$TMP/bin/codesign"

# new_app <dir> → a bundle laid out like the release's, a requirement file per srv.
new_app() {
    local app="$1/AgentMux.app"
    mkdir -p "$app/Contents/Helpers/AgentMux Server.app" "$app/Contents/MacOS" "$1/dr"
    : > "$app/Contents/MacOS/agentmux-srv-0.0.1-darwin.arm64"
    printf '%s' "$GOOD" > "$1/dr/AgentMux Server.app"
    printf '%s' "$GOOD" > "$1/dr/agentmux-srv-0.0.1-darwin.arm64"
}

# run <name> <expected exit> <stderr substring or ""> <app dir> [team override]
run() {
    local name="$1" want="$2" needle="$3" dir="$4" team="${5-$TEAM}" out rc
    out="$(FAKE_DR="$dir/dr" CODESIGN="$TMP/bin/codesign" EXPECTED_TEAM_ID="$team" bash "$SCRIPT" "$dir/AgentMux.app" "unused cert" 2>&1)"; rc=$?
    if [ "$rc" != "$want" ]; then
        bad "$name" "exit $rc, wanted $want: $out"
    elif [ -n "$needle" ] && [[ "$out" != *"$needle"* ]]; then
        bad "$name" "output lacks '$needle': $out"
    else
        ok "$name"
    fi
}

d="$TMP/good"; new_app "$d"
run "the fixed identifier and the signing team pass" 0 "ai.agentmux.srv" "$d"

d="$TMP/helper-id"; new_app "$d"
printf '%s' "${GOOD/ai.agentmux.srv/ai.agentmux.srv-0.0.1}" > "$d/dr/AgentMux Server.app"
run "the helper app signed under another identifier fails, naming it" 1 "identifier found: ai.agentmux.srv-0.0.1" "$d"

d="$TMP/versioned"; new_app "$d"
printf '%s' "${GOOD/ai.agentmux.srv/agentmux-srv-0.0.1-darwin.arm64}" > "$d/dr/agentmux-srv-0.0.1-darwin.arm64"
run "an agentmux-srv* file signed under its file name fails, naming the file" 1 "agentmux-srv-0.0.1-darwin.arm64" "$d"

d="$TMP/lookalike"; new_app "$d"
printf '%s' "${GOOD/ai.agentmux.srv\"/ai.agentmux.srvx\"}" > "$d/dr/AgentMux Server.app"
run "an identifier that only starts with the fixed one fails" 1 "ai.agentmux.srvx" "$d"

d="$TMP/team"; new_app "$d"
run "another team's signature fails" 1 "designated requirement" "$d" "ZZZZZ99999"

d="$TMP/adhoc"; new_app "$d"
printf 'cdhash H"0123456789abcdef0123456789abcdef01234567"' > "$d/dr/AgentMux Server.app"
run "an ad hoc requirement (no identifier, no anchor) fails" 1 "identifier found: <none>" "$d"

d="$TMP/unsigned"; new_app "$d"
rm "$d/dr/agentmux-srv-0.0.1-darwin.arm64"
run "an unsigned srv fails" 1 "designated requirement: <none>" "$d"

d="$TMP/no-helper"; new_app "$d"
rmdir "$d/AgentMux.app/Contents/Helpers/AgentMux Server.app"
run "a bundle without the srv helper app fails (the check can't go quiet)" 1 "is missing" "$d"

d="$TMP/no-symlink"; new_app "$d"
rm "$d/AgentMux.app/Contents/MacOS/agentmux-srv-0.0.1-darwin.arm64"
run "the helper app alone is enough" 0 "" "$d"

run "no readable team fails" 1 "can't read the signing certificate's team" "$TMP/good" ""
run "a malformed team fails" 1 "can't read the signing certificate's team" "$TMP/good" "not*a*team"

# The real thing: a copy signed ad hoc with the right identifier has no
# anchor, so the keychain would not trust it as the release's srv.
if ! command -v codesign > /dev/null 2>&1; then
    printf '  SKIP  a real ad hoc signature (no codesign; macOS only)\n'
else
    d="$TMP/real"; mkdir -p "$d/AgentMux.app/Contents/Helpers/AgentMux Server.app/Contents/MacOS"
    cp /usr/bin/true "$d/AgentMux.app/Contents/Helpers/AgentMux Server.app/Contents/MacOS/agentmux-srv"
    codesign --force --sign - --identifier ai.agentmux.srv "$d/AgentMux.app/Contents/Helpers/AgentMux Server.app/Contents/MacOS/agentmux-srv" > /dev/null 2>&1
    out="$(EXPECTED_TEAM_ID="$TEAM" bash "$SCRIPT" "$d/AgentMux.app" "unused cert" 2>&1)"; rc=$?
    if [ "$rc" = 1 ] && [[ "$out" == *"AgentMux Server.app"* ]]; then
        ok "a real ad hoc signature under ai.agentmux.srv is refused"
    else
        bad "a real ad hoc signature under ai.agentmux.srv is refused" "exit $rc: $out"
    fi
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
