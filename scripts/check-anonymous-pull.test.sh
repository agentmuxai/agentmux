#!/usr/bin/env bash
# check-anonymous-pull.test.sh -- tests for check-anonymous-pull.sh, with `curl`
# replaced by a fake that plays back a registry's answers. No network.
#
# Usage: bash scripts/check-anonymous-pull.test.sh   (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GATE="$HERE/check-anonymous-pull.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

mkdir -p "$TMP/bin"
# The fake curl is driven by env vars:
#   FAKE_FIRST     status for the manifest request with no Authorization header
#   FAKE_AFTER     status for the manifest request with a bearer token
#   FAKE_TOKEN     "yes" to serve a token body from the token endpoint
#   FAKE_CHALLENGE "yes" to send a Bearer challenge with a 401
cat > "$TMP/bin/curl" <<'FAKE'
#!/usr/bin/env bash
args="$*"
if [[ "$args" == *"/token"* ]]; then
    [ "${FAKE_TOKEN:-}" = "yes" ] && printf '{"token":"abc123"}' || printf '{"errors":[]}'
    exit 0
fi
if [[ "$args" == *" -I "* || "$args" == "-s -I"* ]]; then
    if [ "${FAKE_CHALLENGE:-}" = "yes" ]; then
        printf 'HTTP/2 401\r\nwww-authenticate: Bearer realm="https://reg.example/token",service="reg.example",scope="repository:o/r:pull"\r\n\r\n'
    else
        printf 'HTTP/2 401\r\n\r\n'
    fi
    exit 0
fi
if [[ "$args" == *"Authorization: Bearer abc123"* ]]; then
    printf '%s' "${FAKE_AFTER:-000}"
else
    printf '%s' "${FAKE_FIRST:-000}"
fi
FAKE
chmod +x "$TMP/bin/curl"

run() {  # run <want-exit> <name> [ENV=VAL ...]
    local want="$1" name="$2"; shift 2
    local out got
    out="$(env "$@" PATH="$TMP/bin:$PATH" bash "$GATE" reg.example/o/r:latest https://example/settings 2>&1)"; got=$?
    if [ "$got" = "$want" ]; then ok "$name"; else bad "$name" "exit $got, want $want: $out"; fi
    LAST_OUT="$out"
}

echo "check-anonymous-pull.sh"
run 0 "a public image answers 200 straight away" FAKE_FIRST=200
run 0 "a public image behind the token flow" FAKE_FIRST=401 FAKE_CHALLENGE=yes FAKE_TOKEN=yes FAKE_AFTER=200
run 1 "a private image: token granted, manifest refused" FAKE_FIRST=401 FAKE_CHALLENGE=yes FAKE_TOKEN=yes FAKE_AFTER=401
run 1 "a private image: manifest hidden as 404" FAKE_FIRST=401 FAKE_CHALLENGE=yes FAKE_TOKEN=yes FAKE_AFTER=404
run 1 "a 401 with no challenge cannot be pulled" FAKE_FIRST=401
run 1 "a refusal outright" FAKE_FIRST=403
case "$LAST_OUT" in
    *"Danger Zone"*"Public"*) ok "the refusal names the UI step" ;;
    *) bad "the refusal names the UI step" "$LAST_OUT" ;;
esac
case "$LAST_OUT" in
    *https://example/settings*) ok "the refusal links the settings page" ;;
    *) bad "the refusal links the settings page" "$LAST_OUT" ;;
esac
run 2 "a server error is not a verdict" FAKE_FIRST=503
run 2 "no answer at all is not a verdict" FAKE_FIRST=000

n=$(env PATH="$TMP/bin:$PATH" bash "$GATE" 2>&1 >/dev/null; echo $?)
case "$n" in *2) ok "no image argument is a usage error" ;; *) bad "no image argument is a usage error" "$n" ;; esac

echo
echo "$pass passed, $fail failed"
[ "$fail" = "0" ]
