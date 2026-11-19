#!/usr/bin/env bash
# verify-webgpu-libs.test.sh: tests for scripts/verify-webgpu-libs.sh.
#
# Usage: bash scripts/verify-webgpu-libs.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/verify-webgpu-libs.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0

ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

make_lib() {
  local path="$1" size="$2" symbol="$3"
  mkdir -p "$(dirname "$path")"
  {
    [ -n "$symbol" ] && printf '%s\0' "$symbol"
    head -c "$size" /dev/zero | tr '\0' 'x'
  } > "$path"
}

# 1. A real-shaped dxcompiler.dll passes.
d1="$TMP/case1"; make_lib "$d1/dxcompiler.dll" 24000000 "DxcCreateInstance"
if bash "$SCRIPT" "$d1" dxcompiler.dll >/dev/null 2>&1; then
  ok "real-shaped dxcompiler.dll passes"
else
  bad "real-shaped dxcompiler.dll passes" "expected exit 0"
fi

# 2. The reported bug: the file is not in the bundle at all -> must FAIL.
d2="$TMP/case2"; mkdir -p "$d2"; make_lib "$d2/d3dcompiler_47.dll" 4000000 ""
if bash "$SCRIPT" "$d2" dxcompiler.dll >/dev/null 2>&1; then
  bad "a bundle without dxcompiler.dll fails" "expected non-zero exit, got 0"
else
  ok "a bundle without dxcompiler.dll fails"
fi

# 3. An empty or truncated copy fails.
d3="$TMP/case3"; make_lib "$d3/dxcompiler.dll" 40 "DxcCreateInstance"
if bash "$SCRIPT" "$d3" dxcompiler.dll >/dev/null 2>&1; then
  bad "a truncated dxcompiler.dll fails" "expected non-zero exit, got 0"
else
  ok "a truncated dxcompiler.dll fails"
fi

# 4. A large file that does not export the entry point fails.
d4="$TMP/case4"; make_lib "$d4/dxcompiler.dll" 24000000 ""
if bash "$SCRIPT" "$d4" dxcompiler.dll >/dev/null 2>&1; then
  bad "a large file without the export fails" "expected non-zero exit, got 0"
else
  ok "a large file without the export fails"
fi

# 5. Every named library is checked: dxcompiler present, dxil missing -> FAIL.
d5="$TMP/case5"; make_lib "$d5/dxcompiler.dll" 24000000 "DxcCreateInstance"
if bash "$SCRIPT" "$d5" dxcompiler.dll dxil.dll >/dev/null 2>&1; then
  bad "a missing second named library fails" "expected non-zero exit, got 0"
else
  ok "a missing second named library fails"
fi

# 6. Both present and real-shaped pass.
d6="$TMP/case6"; make_lib "$d6/dxcompiler.dll" 24000000 "DxcCreateInstance"
make_lib "$d6/dxil.dll" 1500000 "DxcCreateInstance"
if bash "$SCRIPT" "$d6" dxcompiler.dll dxil.dll >/dev/null 2>&1; then
  ok "both libraries present pass"
else
  bad "both libraries present pass" "expected exit 0"
fi

echo
echo "verify-webgpu-libs.test.sh: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
