#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# prune-dev-builds.test.sh — tests for scripts/prune-dev-builds.sh.
#
# The failure being guarded: a cleanup that meant to keep the running dev build
# deleted its files instead (the kept folder's name carried a trailing slash from
# `ls -d`, so the exclusion matched nothing), and the old Taskfile comment warns
# that `rm -rf` on a live folder deletes what isn't locked. The script must
# delete a build only when no process runs from it.
#
# Usage: bash scripts/prune-dev-builds.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SUT="$HERE/prune-dev-builds.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
PASS=0
FAIL=0

ok() { PASS=$((PASS + 1)); }
bad() { FAIL=$((FAIL + 1)); echo "FAIL: $1"; }

DIST="$TMP/dist"
PROCS="$TMP/procs.txt"

# A fresh dist with the given build folders, each holding a fake app.
setup() {
  rm -rf "$DIST"
  mkdir -p "$DIST/bin" "$DIST/cef"
  for n in "$@"; do
    mkdir -p "$DIST/$n/runtime"
    echo x >"$DIST/$n/runtime/agentmux-cef.exe"
  done
  : >"$PROCS"
}

# The running-process entry for a build, in the form the script compares.
running_from() { # <name>
  local p="$DIST/$1/runtime/agentmux-cef.exe"
  if command -v cygpath >/dev/null 2>&1; then p="$(cygpath -w "$p")"; else p="${p//\//\\}"; fi
  printf '%s\n' "$(printf '%s' "$p" | tr 'A-Z' 'a-z')" >>"$PROCS"
}

# Something else is always running, so "no match" is distinguishable from "no list".
other_process() { echo 'c:\windows\system32\svchost.exe' >>"$PROCS"; }

run() { PRUNE_DIST_DIR="$DIST" PRUNE_PROCESS_LIST="$PROCS" bash "$SUT" "$@" 2>&1; }

exists() { [ -d "$DIST/$1" ]; }

# 1. Idle builds are deleted; dist/bin, dist/cef and the kept build stay.
setup cef-dev-1 cef-dev-2 cef-dev-3
other_process
out="$(run --keep cef-dev-3)"
{ ! exists cef-dev-1 && ! exists cef-dev-2; } && ok || bad "idle builds are deleted ($out)"
exists cef-dev-3 && ok || bad "the --keep build stays"
{ exists bin && exists cef; } && ok || bad "dist/bin and dist/cef are never touched"

# 2. A build with a running process is left whole, even when it is not --keep.
setup cef-dev-1 cef-dev-2
other_process
running_from cef-dev-1
run >/dev/null
exists cef-dev-1 && [ -f "$DIST/cef-dev-1/runtime/agentmux-cef.exe" ] && ok || bad "a running build keeps every file"
! exists cef-dev-2 && ok || bad "an idle build next to a running one is deleted"

# 3. cef-dev-1 is not cef-dev-10: the running process under cef-dev-10 doesn't protect cef-dev-1.
setup cef-dev-1 cef-dev-10
other_process
running_from cef-dev-10
run >/dev/null
! exists cef-dev-1 && ok || bad "a similar name doesn't protect an idle build"
exists cef-dev-10 && ok || bad "the build with the process stays"

# 4. If the process list can't be read, nothing is deleted.
setup cef-dev-1 cef-dev-2
out="$(run)"
{ exists cef-dev-1 && exists cef-dev-2; } && ok || bad "an unreadable process list deletes nothing"
case "$out" in *"deleting nothing"*) ok ;; *) bad "an unreadable process list says so ($out)" ;; esac

# 5. --dry-run deletes nothing and names what it would.
setup cef-dev-1
other_process
out="$(run --dry-run)"
exists cef-dev-1 && ok || bad "--dry-run deletes nothing"
case "$out" in *"would delete cef-dev-1"*) ok ;; *) bad "--dry-run names the build ($out)" ;; esac

# 6. Folders that are not dev builds are never touched.
setup cef-dev-1
other_process
mkdir -p "$DIST/cef-dev" "$DIST/cef-dev-notes-file-not-dir" "$DIST/frontend"
touch "$DIST/cef-dev-file"
run >/dev/null
{ exists cef-dev && exists frontend && [ -f "$DIST/cef-dev-file" ]; } && ok || bad "only cef-dev-<name> folders are candidates ('cef-dev' without a dash is the Unix folder)"

# 6b. A clone on a `subst` drive: the process path and the folder we hold can be
# spelled differently (z:\... vs the physical path behind it), either way round.
winlow() { # <posix path> -> lowercase Windows form, no trailing slash
  local p="$1"
  if command -v cygpath >/dev/null 2>&1; then p="$(cygpath -w "$p")"; else p="${p//\//\\}"; fi
  printf '%s' "$p" | tr 'A-Z' 'a-z'
}
SUBST="$TMP/subst.txt"
printf 'Z:\\: => %s\r\n' "$(winlow "$TMP")" >"$SUBST"

setup cef-dev-1 cef-dev-2
other_process
printf '%s\n' 'z:\dist\cef-dev-1\runtime\agentmux-cef.exe' >>"$PROCS"
PRUNE_SUBST="$SUBST" run >/dev/null
{ exists cef-dev-1 && [ -f "$DIST/cef-dev-1/runtime/agentmux-cef.exe" ]; } && ok || bad "a build running from its subst-drive path stays whole"
! exists cef-dev-2 && ok || bad "an idle build is still deleted when subst drives exist"

out="$(PRUNE_SUBST="$SUBST" PRUNE_PRINT_PREFIXES="z:/dist/cef-dev-1" bash "$SUT" 2>&1)"
case "$out" in *"$(winlow "$TMP")\\dist\\cef-dev-1\\"*) ok ;; *) bad "a subst-drive folder also matches its physical path ($out)" ;; esac
out="$(PRUNE_SUBST="$SUBST" PRUNE_PRINT_PREFIXES="$DIST/cef-dev-1" bash "$SUT" 2>&1)"
case "$out" in *'z:\dist\cef-dev-1\'*) ok ;; *) bad "a physical folder also matches its subst-drive path ($out)" ;; esac
out="$(PRUNE_SUBST="$SUBST" PRUNE_PRINT_PREFIXES="$TMP-elsewhere/dist/cef-dev-1" bash "$SUT" 2>&1)"
case "$out" in *'z:\'*) bad "an unrelated folder doesn't gain a subst spelling ($out)" ;; *) ok ;; esac

# 6c. --only touches one build and nothing else; --grace spares recent builds.
age() { touch -d "$2" "$DIST/$1" 2>/dev/null || touch -t "$(date -d "$2" +%Y%m%d%H%M 2>/dev/null)" "$DIST/$1"; }
setup cef-dev-1 cef-dev-2 cef-dev-3
other_process
run --only cef-dev-2 >/dev/null
{ exists cef-dev-1 && ! exists cef-dev-2 && exists cef-dev-3; } && ok || bad "--only deletes just the named build"

setup cef-dev-1 cef-dev-2
other_process
running_from cef-dev-2
run --only cef-dev-2 >/dev/null
exists cef-dev-2 && [ -f "$DIST/cef-dev-2/runtime/agentmux-cef.exe" ] && ok || bad "--only still leaves a build with a running process whole"

for bad_only in not-a-build ../cef-dev-1 'cef-dev-1/x' 'cef-dev-1\x'; do
  setup cef-dev-1
  other_process
  PRUNE_DIST_DIR="$DIST" PRUNE_PROCESS_LIST="$PROCS" bash "$SUT" --only "$bad_only" >/dev/null 2>&1
  { [ $? = 2 ] && exists cef-dev-1; } && ok || bad "--only '$bad_only' is refused and deletes nothing"
done

setup cef-dev-old cef-dev-new
other_process
age cef-dev-old "3 hours ago"
run --grace 10 >/dev/null
{ ! exists cef-dev-old && exists cef-dev-new; } && ok || bad "--grace deletes an old idle build and spares a fresh one"

setup cef-dev-new
other_process
run --only cef-dev-new --grace 10 >/dev/null
! exists cef-dev-new && ok || bad "--grace doesn't protect the run's own build under --only"

setup cef-dev-1
other_process
PRUNE_DIST_DIR="$DIST" PRUNE_PROCESS_LIST="$PROCS" bash "$SUT" --grace soon >/dev/null 2>&1
[ $? = 2 ] && exists cef-dev-1 && ok || bad "a non-numeric --grace exits 2 and deletes nothing"

# 7. A missing dist is not an error.
rm -rf "$DIST"
PRUNE_DIST_DIR="$DIST" PRUNE_PROCESS_LIST="$PROCS" bash "$SUT" >/dev/null 2>&1 && ok || bad "a missing dist exits 0"

# 8. A bad argument fails loudly.
setup cef-dev-1
other_process
PRUNE_DIST_DIR="$DIST" PRUNE_PROCESS_LIST="$PROCS" bash "$SUT" --nope >/dev/null 2>&1
[ $? = 2 ] && ok || bad "an unknown argument exits 2"
exists cef-dev-1 && ok || bad "a bad argument deletes nothing"

echo "prune-dev-builds: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
