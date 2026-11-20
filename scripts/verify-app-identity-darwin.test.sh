#!/usr/bin/env bash
# verify-app-identity-darwin.test.sh — tests for scripts/verify-app-identity-darwin.sh.
#
# Fake bundles (an Info.plist and, when wanted, the Login Items agent plist) and a fake codesign that prints the
# requirement in $FAKE_DR, so they run anywhere.
#
# Usage: bash scripts/verify-app-identity-darwin.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/verify-app-identity-darwin.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

mkdir -p "$TMP/bin"
cat > "$TMP/bin/codesign" <<'FAKE'
#!/usr/bin/env bash
for last; do :; done
echo "Executable=$last" >&2
[ -n "${FAKE_DR:-}" ] && echo "designated => $FAKE_DR"
exit 0
FAKE
chmod +x "$TMP/bin/codesign"

# new_app <dir> <bundle id> [agent=yes|no] → a bundle with that id (the key and string on separate lines, as plists often are).
new_app() {
    local app="$1/AgentMux.app" id="$2" agent="${3:-yes}"
    mkdir -p "$app/Contents/Library/LaunchAgents"
    printf '<?xml version="1.0"?>\n<plist version="1.0">\n<dict>\n    <key>CFBundleName</key><string>AgentMux</string>\n    <key>CFBundleIdentifier</key>\n    <string>%s</string>\n</dict>\n</plist>\n' "$id" > "$app/Contents/Info.plist"
    [ "$agent" = "yes" ] && : > "$app/Contents/Library/LaunchAgents/$id.login.plist"
    return 0
}

# run <name> <expected exit> <output substring or ""> <app dir> [--signed] [requirement]
run() {
    local name="$1" want="$2" needle="$3" dir="$4" mode="${5-}" dr="${6-}" out rc
    out="$(FAKE_DR="$dr" CODESIGN="$TMP/bin/codesign" bash "$SCRIPT" "$dir/AgentMux.app" $mode 2>&1)"; rc=$?
    if [ "$rc" != "$want" ]; then
        bad "$name" "exit $rc, wanted $want: $out"
    elif [ -n "$needle" ] && [[ "$out" != *"$needle"* ]]; then
        bad "$name" "output lacks '$needle': $out"
    else
        ok "$name"
    fi
}

GOOD='identifier "ai.agentmux.stable" and anchor apple generic and certificate leaf[subject.OU] = "ABCDE12345"'

d="$TMP/stable"; new_app "$d" ai.agentmux.stable
run "the fixed stable id passes" 0 "bundle id: ai.agentmux.stable" "$d"
run "the fixed stable id passes signed, with its requirement" 0 "designated requirement names" "$d" --signed "$GOOD"

d="$TMP/versioned"; new_app "$d" ai.agentmux.stable.0.60.36
run "a stable id with a version fails, naming it" 1 "ai.agentmux.stable.0.60.36" "$d"

d="$TMP/other-suffix"; new_app "$d" ai.agentmux.stable.beta
run "a stable id with any other suffix fails" 1 "not 'ai.agentmux.stable'" "$d"

d="$TMP/local"; new_app "$d" ai.agentmux.local-main-b28b7a-c2426d68.0.60.1 no
run "another channel's per-version id is left alone" 0 "left as it is" "$d"

d="$TMP/no-agent"; new_app "$d" ai.agentmux.stable no
run "the stable app without its Login Items agent plist fails" 1 "login.plist is missing" "$d"

d="$TMP/signed-other"; new_app "$d" ai.agentmux.stable
run "a signed requirement that names a versioned id fails" 1 "ai.agentmux.stable.0.60.36" "$d" --signed 'identifier "ai.agentmux.stable.0.60.36" and anchor apple generic'

d="$TMP/signed-adhoc"; new_app "$d" ai.agentmux.stable
run "an ad hoc requirement (no Developer ID anchor) fails" 1 "<none>" "$d" --signed ""

d="$TMP/no-plist"; mkdir -p "$d/AgentMux.app/Contents"
run "an app without an Info.plist fails" 1 "no Info.plist" "$d"

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
