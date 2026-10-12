#!/usr/bin/env bash
# verify-cef-framework-darwin.test.sh — tests for scripts/verify-cef-framework-darwin.sh.
#
# The script reads a framework's symbol table with `nm`; these tests put a fake
# `nm` on PATH that prints a fixed table, so they run anywhere (no 489 MB
# framework, no macOS). Case "stale fork build" is the one that went wrong for
# real: a framework built from the fork before agentmuxai/cef#11 carries
# BeginWindowDrag, passed the old check, and crashed the host the first time a
# browser pane opened in a second window ("Observers can only be added once!").
#
# Usage: bash scripts/verify-cef-framework-darwin.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/verify-cef-framework-darwin.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0

ok()   { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad()  { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

mkdir -p "$TMP/bin"
cat > "$TMP/bin/nm" <<'EOF'
#!/usr/bin/env bash
cat "$FAKE_NM_TABLE"
EOF
chmod +x "$TMP/bin/nm"
BIN="$TMP/Chromium Embedded Framework"
: > "$BIN"

# Symbol lines as nm prints them; lowercase type = local symbol.
DRAG='000000000011a000 t __ZN13CefWindowImpl15BeginWindowDragEv'
THEME='000000000011d1a8 t __ZNSt4__Cr6__treeINS_12__value_typeIN4base7raw_ptrI12ThemeServiceLN15partition_alloc8internal12RawPtrTraitsE0EEEmEENS_19__map_value_compareIS8_NS_4pairIKS8_mEE'
OTHER_LOCAL='0000000000100000 t __ZN13CefWidgetImpl14OnThemeChangedEv'
EXTERNAL='0000000000001000 T _cef_initialize'

# run <name> <expected exit> <stderr substring or ""> <symbol lines...>
run() {
    local name="$1" want="$2" needle="$3"; shift 3
    printf '%s\n' "$@" > "$TMP/table"
    local err code
    err="$(PATH="$TMP/bin:$PATH" FAKE_NM_TABLE="$TMP/table" bash "$SCRIPT" "$BIN" 2>&1 >/dev/null)"
    code=$?
    if [ "$code" != "$want" ]; then
        bad "$name" "exit $code, want $want; stderr: $err"
    elif [ -n "$needle" ] && ! grep -qF -- "$needle" <<<"$err"; then
        bad "$name" "stderr lacks '$needle': $err"
    else
        ok "$name"
    fi
}

run "current fork build (both patches)"        0 ""                    "$EXTERNAL" "$OTHER_LOCAL" "$DRAG" "$THEME"
run "stale fork build (no cef#11)"             1 "agentmuxai/cef#11"   "$EXTERNAL" "$OTHER_LOCAL" "$DRAG"
run "upstream CEF (neither patch)"             1 "BeginWindowDrag"     "$EXTERNAL" "$OTHER_LOCAL"
run "cef#11 but no BeginWindowDrag"            1 "BeginWindowDrag"     "$EXTERNAL" "$OTHER_LOCAL" "$THEME"
run "stripped framework (no local symbols)"    2 "STRIPPED"            "$EXTERNAL"

err="$(PATH="$TMP/bin:$PATH" FAKE_NM_TABLE=/dev/null bash "$SCRIPT" "$TMP/missing" 2>&1 >/dev/null)"; code=$?
if [ "$code" = 2 ]; then ok "missing binary"; else bad "missing binary" "exit $code: $err"; fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
