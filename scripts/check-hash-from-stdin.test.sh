#!/usr/bin/env bash
# check-hash-from-stdin.test.sh — tests for check-hash-from-stdin.sh.
#
# Usage: bash scripts/check-hash-from-stdin.test.sh   (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GATE="$HERE/check-hash-from-stdin.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

n=0
expect() {  # expect <want-exit> <name> <line>
    n=$((n+1)); local f="$TMP/case$n.sh"
    printf '%s\n' "$3" > "$f"
    out="$(bash "$GATE" "$f" 2>&1)"; got=$?
    if [ "$got" = "$1" ]; then ok "$2"; else bad "$2" "exit $got, want $1: $out"; fi
}

echo "check-hash-from-stdin.sh"
# Violations: a file name reaches the hash command and its output is parsed.
expect 1 "sha256sum \"\$f\" | cut (the verify-cef-runtime-windows.sh bug)" \
    'actual="$(sha256sum "$libcef" | cut -d'"' '"' -f1)"'
expect 1 "unquoted file argument" 'h=$(sha256sum $f | cut -c1-12)'
expect 1 "shasum -a 256 <file> | awk" 'h=$(shasum -a 256 "$f" | awk '"'"'{print $1}'"'"')'
expect 1 "md5sum <file> | cut" 'k=$(md5sum "$url_file" | cut -c1-8)'
expect 1 "indented, inside an if" '    if [ "$(sha256sum "$RCEDIT" | cut -d'"' '"' -f1)" != "$SHA" ]; then'

# Clean.
expect 0 "stdin redirect" 'actual="$(sha256sum < "$libcef" | cut -d'"' '"' -f1)"'
expect 0 "piped input" 'id=$(printf '"'"'%s'"'"' "$label" | shasum -a 1 | cut -c1-8)'
expect 0 "echo | md5sum | cut" 'k="$(echo "$url" | md5sum | cut -c1-8)"'
expect 0 "shasum -a 256 < file | cut" 'h=$(shasum -a 256 < "$f" | cut -d'"' '"' -f1)'
expect 0 "sha256sum -c (verifying, not parsing)" 'sha256sum -c SHA256SUMS.txt'
expect 0 "hash printed for a human, no pipe" 'sha256sum "$artifact"'
expect 0 "comment line" '# actual="$(sha256sum "$libcef" | cut -d'"' '"' -f1)"'
expect 0 "name merely containing sha256sum" 'my_sha256sum_helper "$f" | tee log'

echo "repo"
out="$(bash "$GATE" 2>&1)"; got=$?
if [ "$got" = 0 ]; then ok "the repo itself is clean"; else bad "the repo itself is clean" "$out"; fi

echo "$pass passed, $fail failed"
[ "$fail" = 0 ]
