#!/usr/bin/env bash
# verify-angle-libs.test.sh — tests for scripts/verify-angle-libs.sh.
#
# Regression coverage for a real false positive (agentmuxai/agentmux#3172):
# the first revision of verify-angle-libs.sh used a uniform 1MB size floor
# for both libEGL and libGLESv2, and returned early on a small file WITHOUT
# ever running the symbol check. libEGL.dll is legitimately ~500KB even in
# a fully real, functional ANGLE build (it's a thin dispatch layer; the bulk
# of the real code is in libGLESv2), so that floor false-positived on a
# genuine, working CEF 152 Windows build — measured: a real libEGL.dll was
# 510,464 bytes, and a known-broken dummy_stub.cc placeholder was 471,552
# bytes, an 8% difference no fixed size threshold can safely separate.
#
# Usage: bash scripts/verify-angle-libs.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/verify-angle-libs.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0

ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

# A real symbol table is much bigger than just the target string — pad with
# filler so size-based checks see something plausible either way.
make_lib() {
  local path="$1" size="$2" symbol="$3"
  mkdir -p "$(dirname "$path")"
  {
    [ -n "$symbol" ] && printf '%s\0' "$symbol"
    head -c "$size" /dev/zero | tr '\0' 'x'
  } > "$path"
}

# 1. Small (510KB-scale) but genuinely has the export -> must PASS. This is
#    the exact regression: the original script never even reached the grep
#    for a file this size.
d1="$TMP/case1"; make_lib "$d1/libEGL.dll" 510464 "eglGetProcAddress"
make_lib "$d1/libGLESv2.dll" 8400000 "glGetString"
if bash "$SCRIPT" "$d1" libEGL.dll libGLESv2.dll >/dev/null 2>&1; then
  ok "small-but-real libEGL (510KB, has export) passes"
else
  bad "small-but-real libEGL (510KB, has export) passes" "expected exit 0"
fi

# 2. Known-broken-shaped stub: ~470KB, no export string at all -> must FAIL.
d2="$TMP/case2"; make_lib "$d2/libEGL.dll" 470016 ""
make_lib "$d2/libGLESv2.dll" 470016 ""
if bash "$SCRIPT" "$d2" libEGL.dll libGLESv2.dll >/dev/null 2>&1; then
  bad "stub-shaped files with no export fail" "expected non-zero exit, got 0"
else
  ok "stub-shaped files with no export fail"
fi

# 3. Large file but no export string -> must FAIL (size alone is not
#    sufficient evidence of a real build).
d3="$TMP/case3"; make_lib "$d3/libEGL.dll" 2000000 ""
make_lib "$d3/libGLESv2.dll" 8400000 ""
if bash "$SCRIPT" "$d3" libEGL.dll libGLESv2.dll >/dev/null 2>&1; then
  bad "large file without export still fails" "expected non-zero exit, got 0"
else
  ok "large file without export still fails"
fi

# 4. Truly tiny/corrupt file (well under any real build's size) -> must
#    FAIL via the sane-floor, even before the symbol check.
d4="$TMP/case4"; make_lib "$d4/libEGL.dll" 50 "eglGetProcAddress"
make_lib "$d4/libGLESv2.dll" 8400000 "glGetString"
if bash "$SCRIPT" "$d4" libEGL.dll libGLESv2.dll >/dev/null 2>&1; then
  bad "tiny/corrupt file fails the sanity floor" "expected non-zero exit, got 0"
else
  ok "tiny/corrupt file fails the sanity floor"
fi

# 5. Missing files are skipped, not flagged -- caller decides if absence is
#    fatal, per the script's own doc comment.
d5="$TMP/case5"; mkdir -p "$d5"
if bash "$SCRIPT" "$d5" libEGL.dll libGLESv2.dll >/dev/null 2>&1; then
  ok "missing files are skipped (exit 0), not flagged"
else
  bad "missing files are skipped (exit 0), not flagged" "expected exit 0"
fi

# 6. Both real and correctly sized/symboled -> PASS (the ordinary case).
d6="$TMP/case6"; make_lib "$d6/libEGL.dll" 510464 "eglGetProcAddress"
make_lib "$d6/libGLESv2.dll" 8400000 "glGetString"
if bash "$SCRIPT" "$d6" libEGL.dll libGLESv2.dll >/dev/null 2>&1; then
  ok "two genuinely real files pass"
else
  bad "two genuinely real files pass" "expected exit 0"
fi

echo
echo "verify-angle-libs.test.sh: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
