#!/usr/bin/env bash
# Tests for scripts/cef-build/verify-glibc-floor.sh, using the glibc of the machine they run on
# (Linux only): a real ELF that needs GLIBC_x passes a floor of x and fails a floor below it.
#
# Usage: bash scripts/cef-build/verify-glibc-floor.test.sh    (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/verify-glibc-floor.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()  { pass=$((pass + 1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

if ! command -v objdump >/dev/null 2>&1 || [ ! -x /bin/ls ]; then
  echo "skipped: needs objdump and /bin/ls"
  exit 0
fi

mkdir -p "$TMP/rt"
cp /bin/ls "$TMP/rt/libfake.so"
printf 'not an elf file\n' > "$TMP/rt/readme.txt"
need="$(objdump -T /bin/ls | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -Vu | tail -1)"

if bash "$SCRIPT" "$TMP/rt" 99.0 >/dev/null 2>&1; then ok "passes under a high floor"; else bad "passes under a high floor"; fi
if bash "$SCRIPT" "$TMP/rt" "$need" >/dev/null 2>&1; then ok "passes at exactly the version it needs ($need)"; else bad "passes at exactly the version it needs"; fi
out="$(bash "$SCRIPT" "$TMP/rt" 2.0 2>&1)"; code=$?
if [ "$code" -eq 1 ] && printf '%s' "$out" | grep -q 'libfake.so needs GLIBC_'; then ok "fails below it and names the file"; else bad "fails below it and names the file" "code=$code out=$out"; fi

mkdir -p "$TMP/none"; printf 'x\n' > "$TMP/none/a.txt"
bash "$SCRIPT" "$TMP/none" >/dev/null 2>&1; code=$?
if [ "$code" -eq 2 ]; then ok "a directory with no ELF file is an error, not a pass"; else bad "no ELF is an error" "code=$code"; fi
bash "$SCRIPT" "$TMP/missing" >/dev/null 2>&1; code=$?
if [ "$code" -eq 2 ]; then ok "a missing directory is an error"; else bad "a missing directory is an error" "code=$code"; fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
