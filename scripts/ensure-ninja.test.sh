#!/usr/bin/env bash
# ensure-ninja.test.sh — tests for scripts/ensure-ninja.sh.
#
# The regression this pins (Kanban #31442): WinGet installs ninja as a "Links"
# symlink alias. Git Bash lists it, but native CMake cannot start it, so
# `task dev` died inside cef-dll-sys with exit status 58 and no mention of
# ninja. The script must find a real ninja and print the directory to prepend
# to PATH; remove a cef-dll-sys CMakeCache.txt that pinned the unusable path;
# and fail with instructions when nothing usable exists.
#
# Usage: bash scripts/ensure-ninja.test.sh    (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/ensure-ninja.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

pass=0
fail=0
ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

# Force the Windows branch on any host, so the test does not need Windows and
# the host's own ninja/Visual Studio can never decide the result.
mkdir -p "$TMP/bin"
cat > "$TMP/bin/uname" <<'EOF'
#!/bin/sh
echo MINGW64_NT-10.0
EOF
chmod +x "$TMP/bin/uname"

# A real ninja is any executable that answers --version.
mkdir -p "$TMP/real"
printf '#!/bin/sh\necho 1.13.2\n' > "$TMP/real/ninja"
cp "$TMP/real/ninja" "$TMP/real/ninja.exe"
chmod +x "$TMP/real/ninja" "$TMP/real/ninja.exe"

# A broken one sits on PATH but cannot start (the WinGet alias's behaviour).
mkdir -p "$TMP/broken"
printf '#!/bin/sh\nexit 1\n' > "$TMP/broken/ninja"
cp "$TMP/broken/ninja" "$TMP/broken/ninja.exe"
chmod +x "$TMP/broken/ninja" "$TMP/broken/ninja.exe"

mkdir -p "$TMP/empty" "$TMP/home"

# A usable ninja on PATH needs no fix: no output, exit 0.
out=$(PATH="$TMP/bin:$TMP/real:/usr/bin:/bin" HOME="$TMP/home" \
      AGENTMUX_NINJA_SEARCH="$TMP/empty" AGENTMUX_TARGET_DIR="$TMP/target" \
      "$SCRIPT" 2>"$TMP/err1"); rc=$?
if [ "$rc" -eq 0 ] && [ -z "$out" ]; then
  ok "a usable ninja on PATH needs no fix"
else
  bad "a usable ninja on PATH needs no fix" "rc=$rc out='$out' $(cat "$TMP/err1")"
fi

# A broken ninja on PATH, with a real one in the search path: print its dir.
out=$(PATH="$TMP/bin:$TMP/broken:/usr/bin:/bin" HOME="$TMP/home" \
      AGENTMUX_NINJA_SEARCH="$TMP/real" AGENTMUX_TARGET_DIR="$TMP/target" \
      "$SCRIPT" 2>"$TMP/err2"); rc=$?
if [ "$rc" -eq 0 ] && [ "$out" = "$TMP/real" ]; then
  ok "a broken ninja is replaced by a real one from the search path"
else
  bad "a broken ninja is replaced by a real one from the search path" "rc=$rc out='$out' $(cat "$TMP/err2")"
fi

# A symlink alias whose target is real: the target's directory is printed.
# Git Bash's `ln -s` copies instead of symlinking unless Windows is configured
# for native symlinks, so skip when no symlink was made (the real WinGet alias
# is a reparse point; readlink -f resolves it, as the script already does).
mkdir -p "$TMP/links"
ln -s "$TMP/real/ninja" "$TMP/links/ninja"
if [ -L "$TMP/links/ninja" ]; then
  out=$(PATH="$TMP/bin:$TMP/links:/usr/bin:/bin" HOME="$TMP/home" \
        AGENTMUX_TARGET_DIR="$TMP/target" \
        "$SCRIPT" 2>"$TMP/err2b"); rc=$?
  if [ "$rc" -eq 0 ] && [ "$out" = "$TMP/real" ]; then
    ok "a symlink alias resolves to its real target"
  else
    bad "a symlink alias resolves to its real target" "rc=$rc out='$out' $(cat "$TMP/err2b")"
  fi
else
  ok "a symlink alias resolves to its real target (skipped: ln -s made a copy)"
fi

# No usable ninja anywhere: exit 1, no PATH line, instructions on stderr.
out=$(PATH="$TMP/bin:$TMP/broken:/usr/bin:/bin" HOME="$TMP/home" \
      AGENTMUX_NINJA_SEARCH="$TMP/empty" AGENTMUX_TARGET_DIR="$TMP/target" \
      "$SCRIPT" 2>"$TMP/err3"); rc=$?
if [ "$rc" -eq 1 ] && [ -z "$out" ] && grep -q 'needs a ninja that starts' "$TMP/err3"; then
  ok "no usable ninja fails with instructions"
else
  bad "no usable ninja fails with instructions" "rc=$rc out='$out' $(cat "$TMP/err3")"
fi

# A cef-dll-sys cache that pinned a path which cannot run is removed; one that
# pinned a working ninja is kept.
mkdir -p "$TMP/target/release/build/cef-dll-sys-bad/out/build"
mkdir -p "$TMP/target/debug/build/cef-dll-sys-good/out/build"
printf 'CMAKE_MAKE_PROGRAM:FILEPATH=%s\n' "$TMP/broken/ninja" > "$TMP/target/release/build/cef-dll-sys-bad/out/build/CMakeCache.txt"
printf 'CMAKE_MAKE_PROGRAM:FILEPATH=%s\n' "$TMP/real/ninja" > "$TMP/target/debug/build/cef-dll-sys-good/out/build/CMakeCache.txt"
out=$(PATH="$TMP/bin:$TMP/broken:/usr/bin:/bin" HOME="$TMP/home" \
      AGENTMUX_NINJA_SEARCH="$TMP/real" AGENTMUX_TARGET_DIR="$TMP/target" \
      "$SCRIPT" 2>"$TMP/err4"); rc=$?
if [ "$rc" -eq 0 ] \
   && [ ! -f "$TMP/target/release/build/cef-dll-sys-bad/out/build/CMakeCache.txt" ] \
   && [ -f "$TMP/target/debug/build/cef-dll-sys-good/out/build/CMakeCache.txt" ] \
   && grep -q 'removed stale CMake cache' "$TMP/err4"; then
  ok "a stale cef-dll-sys CMake cache is removed, a good one kept"
else
  bad "a stale cef-dll-sys CMake cache is removed, a good one kept" "rc=$rc $(cat "$TMP/err4")"
fi

# --check is read-only and names the real ninja.
out=$(PATH="$TMP/bin:$TMP/broken:/usr/bin:/bin" HOME="$TMP/home" \
      AGENTMUX_NINJA_SEARCH="$TMP/real" AGENTMUX_TARGET_DIR="$TMP/target" \
      "$SCRIPT" --check 2>&1); rc=$?
if [ "$rc" -eq 0 ] && grep -q 'OK   ninja' <<<"$out" && grep -q "$TMP/real" <<<"$out"; then
  ok "--check names the real ninja"
else
  bad "--check names the real ninja" "rc=$rc out='$out'"
fi

printf '\n%s passed, %s failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
