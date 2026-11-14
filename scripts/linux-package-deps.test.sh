#!/usr/bin/env bash
# Tests for scripts/linux-package-deps.sh: the list turns into Depends and Requires lines, and
# `check` fails when a staged tree needs a library nobody bundles or lists.
#
#   bash scripts/linux-package-deps.test.sh
#
# The staged trees are built from a real dynamically linked binary of the machine running the
# test (it needs readelf and a `ls`), so no compiler is required.

set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/linux-package-deps.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
FAILED=0

pass() { echo "ok   $1"; }
fail() { echo "FAIL $1"; FAILED=1; }

command -v readelf >/dev/null || { echo "skip: readelf not found"; exit 0; }
BIN="$(command -v ls)"
NEEDED="$(readelf -d "$BIN" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | grep -vE '^(libc\.so\.6|libm\.so\.6|ld-linux-x86-64\.so\.2|libgcc_s\.so\.1)$' | head -1)"
[ -n "$NEEDED" ] || { echo "skip: $BIN needs nothing beyond glibc on this machine"; exit 0; }

mkdir -p "$TMP/root/opt/agentmux/bin"
cp "$BIN" "$TMP/root/opt/agentmux/bin/tool"

# --- the real list: one Debian entry per package set, one soname requirement per library ---
deb="$(bash "$SCRIPT" deb "$TMP/root")"
rpm="$(bash "$SCRIPT" rpm "$TMP/root")"
echo "$deb" | grep -qx 'libasound2t64 | libasound2' && pass "deb: a renamed package lists both names" || fail "deb: libasound2t64 | libasound2 missing"
[ "$(echo "$deb" | grep -c 'libglib2.0-0t64')" = 1 ] && pass "deb: libraries of one package are declared once" || fail "deb: libglib2.0-0t64 repeated or missing"
echo "$rpm" | grep -qx 'libasound.so.2()(64bit)' && pass "rpm: a library is required by soname" || fail "rpm: libasound.so.2()(64bit) missing"
echo "$rpm" | grep -q ' ' && fail "rpm: an entry has a space" || pass "rpm: no entry has a space"
echo "$deb" | grep -qx 'ca-certificates' && pass "deb: the system certificates are required" || fail "deb: ca-certificates missing"
echo "$rpm" | grep -qx 'ca-certificates' && pass "rpm: the system certificates are required" || fail "rpm: ca-certificates missing"

# --- the glibc floor is the highest version the binaries link against, not a constant ---
want="$(readelf -V "$BIN" | sed -n 's/.*Name: GLIBC_\([0-9][0-9.]*\).*/\1/p' | sort -uV | tail -1)"
echo "$deb" | grep -qx "libc6 (>= $want)" && pass "deb: libc6 (>= $want), read from the binary" || fail "deb: expected libc6 (>= $want) in: $deb"
echo "$rpm" | grep -qx "libc.so.6(GLIBC_$want)(64bit)" && pass "rpm: libc.so.6(GLIBC_$want)(64bit), read from the binary" || fail "rpm: expected libc.so.6(GLIBC_$want)(64bit) in: $rpm"
mkdir -p "$TMP/nobin"
[ "$(bash "$SCRIPT" deb "$TMP/nobin" | grep -c '^libc6')" = 0 ] && pass "deb: a tree with no binaries declares no glibc floor" || fail "deb: a glibc floor for an empty tree"

# --- check: a tree that needs a library the list lacks fails and names it ---
printf '# nothing listed\n' > "$TMP/empty.list"
out="$(LINUX_PACKAGE_DEPS_LIST="$TMP/empty.list" bash "$SCRIPT" check "$TMP/root" 2>&1)"; rc=$?
if [ $rc -eq 1 ] && echo "$out" | grep -qF "$NEEDED"; then pass "check: an unlisted library fails and is named ($NEEDED)"; else fail "check: expected rc 1 naming $NEEDED, got rc $rc: $out"; fi

# --- check: listing every needed library makes it pass ---
readelf -d "$BIN" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | sed 's/$/ | somepackage/' > "$TMP/full.list"
out="$(LINUX_PACKAGE_DEPS_LIST="$TMP/full.list" bash "$SCRIPT" check "$TMP/root" 2>&1)"; rc=$?
[ $rc -eq 0 ] && pass "check: every needed library listed passes" || fail "check: expected rc 0, got rc $rc: $out"

# --- check: a library the tree carries itself needs no entry ---
mkdir -p "$TMP/bundled/opt/agentmux/bin"
cp "$BIN" "$TMP/bundled/opt/agentmux/bin/tool"
for lib in $(readelf -d "$BIN" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do
    src="$(ldconfig -p 2>/dev/null | sed -n "s/^[[:space:]]*$lib .*=> //p" | head -1)"
    [ -n "$src" ] && cp -L "$src" "$TMP/bundled/opt/agentmux/bin/$lib"
done
out="$(LINUX_PACKAGE_DEPS_LIST="$TMP/empty.list" bash "$SCRIPT" check "$TMP/bundled" 2>&1)"; rc=$?
# Bundled copies bring their own NEEDED entries; only the tool's own are asserted gone.
echo "$out" | grep -qF "  $NEEDED" && fail "check: a bundled $NEEDED is still reported" || pass "check: a bundled library is not reported"

# --- check: a listed library nothing needs is reported but does not fail ---
{ cat "$TMP/full.list"; echo "libnothing.so.9 | nothing"; } > "$TMP/extra.list"
out="$(LINUX_PACKAGE_DEPS_LIST="$TMP/extra.list" bash "$SCRIPT" check "$TMP/root" 2>&1)"; rc=$?
[ $rc -eq 0 ] && echo "$out" | grep -q 'libnothing.so.9' && pass "check: an unused entry is reported, not fatal" || fail "check: expected rc 0 naming libnothing.so.9, got rc $rc: $out"

# --- usage ---
bash "$SCRIPT" bogus >/dev/null 2>&1; [ $? -eq 2 ] && pass "an unknown command exits 2" || fail "an unknown command should exit 2"

exit $FAILED
