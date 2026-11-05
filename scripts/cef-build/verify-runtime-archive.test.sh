#!/usr/bin/env bash
# Tests for scripts/cef-build/verify-runtime-archive.sh.
#
# Usage: bash scripts/cef-build/verify-runtime-archive.test.sh    (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/verify-runtime-archive.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()  { pass=$((pass + 1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

printf 'a runtime\n' > "$TMP/a.tar.gz"
GOOD="$(sha256sum < "$TMP/a.tar.gz" | cut -d' ' -f1)"
printf 'CEF_LINUX_RUNTIME_TAG="lin-r2"\nCEF_LINUX_RUNTIME_SHA256="%s"\n' "$GOOD" > "$TMP/pins.sh"
printf 'CEF_LINUX_RUNTIME_TAG="lin-r2"\n' > "$TMP/pins-nohash.sh"

if bash "$SCRIPT" "$TMP/a.tar.gz" lin-r2 "$TMP/pins.sh" >/dev/null 2>&1; then ok "the pinned archive passes"; else bad "the pinned archive passes"; fi

printf 'another runtime\n' > "$TMP/b.tar.gz"
out="$(bash "$SCRIPT" "$TMP/b.tar.gz" lin-r2 "$TMP/pins.sh" 2>&1)"; code=$?
if [ "$code" -eq 1 ] && printf '%s' "$out" | grep -q "the pin says $GOOD"; then ok "another archive of the pinned tag fails and names both hashes"; else bad "another archive fails" "code=$code out=$out"; fi

if bash "$SCRIPT" "$TMP/b.tar.gz" lin-other "$TMP/pins.sh" >/dev/null 2>&1; then ok "a tag that is not pinned is not checked"; else bad "a tag that is not pinned is not checked"; fi

bash "$SCRIPT" "$TMP/a.tar.gz" lin-r2 "$TMP/pins-nohash.sh" >/dev/null 2>&1; code=$?
if [ "$code" -eq 1 ]; then ok "a pinned tag without a hash fails"; else bad "a pinned tag without a hash fails" "code=$code"; fi

bash "$SCRIPT" "$TMP/missing.tar.gz" lin-r2 "$TMP/pins.sh" >/dev/null 2>&1; code=$?
if [ "$code" -eq 2 ]; then ok "a missing archive is an error"; else bad "a missing archive is an error" "code=$code"; fi
bash "$SCRIPT" >/dev/null 2>&1; code=$?
if [ "$code" -eq 2 ]; then ok "no arguments is a usage error"; else bad "no arguments is a usage error" "code=$code"; fi

# The committed pin names a hash for its tag.
REAL="$HERE/cef-runtime-pins.sh"
if (source "$REAL"; [ -n "${CEF_LINUX_RUNTIME_SHA256:-}" ] && [ "${#CEF_LINUX_RUNTIME_SHA256}" -eq 64 ]); then ok "the committed Linux pin carries a 64-digit hash"; else bad "the committed Linux pin carries a hash"; fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
