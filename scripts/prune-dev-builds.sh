#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# prune-dev-builds.sh — deletes the old `dist/cef-dev-<epoch>` folders that
# `task dev` leaves behind on Windows, skipping any that a process is running
# from.
#
# WHY: on Windows every `task dev` run makes a fresh `dist/cef-dev-<epoch>` of
# about 1 GB (the app, CEF and srv copied side by side) and nothing removes it.
# Twenty-eight of them were 21 GB on one host. The Taskfile used to say "no
# prune loop here" and for good reason: `rm -rf` on a folder a live app runs
# from succeeds on the unlocked files and fails on the locked ones, leaving the
# running app without its helper tools. So this deletes a build only when no
# process has an executable under it. See
# docs/specs/SPEC_TASK_DEV_PRUNE_OLD_BUILDS_2026_10_14.md.
#
# Usage:
#   prune-dev-builds.sh [--keep <name>] [--wait <seconds>] [--dry-run]
#
#   --keep <name>   never delete dist/<name> (the build `task dev` is starting)
#   --wait <secs>   give processes still running from a build this long to
#                   exit before leaving it for the next run (default 0). The
#                   exit-time call uses it: the app's helper processes take a
#                   few seconds to follow the launcher out.
#   --dry-run       say what would be deleted and delete nothing
#
# Only `dist/cef-dev-*` folders directly under this clone's `dist/` are ever
# touched. Off Windows `task dev` reuses one fixed folder, so this does nothing.
# Whatever it cannot prove is idle, it leaves alone: if the process list can't
# be read, nothing is deleted.
#
# Test seam: PRUNE_PROCESS_LIST=<file> reads the lowercased executable paths
# (one per line, Windows form) from a file instead of asking Windows.

set -uo pipefail

KEEP=""
WAIT=0
DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --keep) KEEP="${2:-}"; shift 2 2>/dev/null || shift ;;
    --wait) WAIT="${2:-0}"; shift 2 2>/dev/null || shift ;;
    --dry-run) DRY=1; shift ;;
    *) echo "prune-dev-builds: unknown argument '$1'" >&2; exit 2 ;;
  esac
done
case "$WAIT" in '' | *[!0-9]*) echo "prune-dev-builds: --wait takes whole seconds" >&2; exit 2 ;; esac

# Lowercased executable path of every running process, one per line. Empty means
# the query failed (powershell itself is always in the list), so callers treat
# it as "unknown" and delete nothing.
process_paths() {
  if [ -n "${PRUNE_PROCESS_LIST:-}" ]; then
    cat "$PRUNE_PROCESS_LIST" 2>/dev/null
    return
  fi
  powershell -NoProfile -Command \
    'Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath } | ForEach-Object { $_.ExecutablePath.ToLowerInvariant() }' \
    2>/dev/null | tr -d '\r'
}

# Active `subst` drives as "<drive>:<TAB><target>" lines, lowercased with
# backslashes: `K:\: => C:\Users\me\clone` becomes `k:<TAB>c:\users\me\clone`.
# Test seam: PRUNE_SUBST=<file> holds `subst`'s output instead.
subst_pairs() {
  if [ -n "${PRUNE_SUBST:-}" ]; then cat "$PRUNE_SUBST" 2>/dev/null; else subst 2>/dev/null; fi |
    tr -d '\r' | tr 'A-Z' 'a-z' | awk -F' => ' 'NF == 2 { d = $1; sub(/\\?:?$/, "", d); print d "\t" $2 }'
}

# Every Windows spelling of a folder, lowercased with a trailing backslash (so a
# prefix match can't confuse cef-dev-17 with cef-dev-170), one per line. The
# running app's executable path can use a different one than the path we hold
# when the clone sits on a `subst` drive (the MAX_PATH workaround in the run
# skill): the folder as given, the physical path behind a subst drive, and the
# subst drive for a physical path.
win_prefixes() {
  local p="$1" drive target
  if command -v cygpath >/dev/null 2>&1; then p="$(cygpath -w "$p")"; fi
  # Backslashes either way: without cygpath (the CI test runs on Linux) the
  # path is POSIX, and the process list is always in Windows form.
  p="${p//\//\\}"
  p="$(printf '%s' "$p" | tr 'A-Z' 'a-z')"
  printf '%s\\\n' "$p"
  while IFS=$'\t' read -r drive target; do
    [ -n "$drive" ] && [ -n "$target" ] || continue
    target="${target%\\}"
    case "$p" in
      "$drive"\\*) printf '%s\\\n' "$target${p#"$drive"}" ;;
    esac
    case "$p" in
      "$target"\\*) printf '%s\\\n' "$drive${p#"$target"}" ;;
    esac
  done <<EOF
$(subst_pairs)
EOF
}

in_use() { # <folder>; reads $PROCS
  local prefix
  # A bash string match, not grep: Git for Windows' grep aborts (SIGABRT) on a
  # fixed-string pattern ending in a backslash, and as a pipeline reader its
  # early exit would trip pipefail. Both sides are already lowercased.
  while IFS= read -r prefix; do
    [ -n "$prefix" ] || continue
    case "$PROCS" in
      *"$prefix"*) return 0 ;;
    esac
  done <<EOF
$(win_prefixes "$1")
EOF
  return 1
}

# Test hook: print the spellings checked for a folder, then stop.
if [ -n "${PRUNE_PRINT_PREFIXES:-}" ]; then win_prefixes "$PRUNE_PRINT_PREFIXES"; exit 0; fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIST="${PRUNE_DIST_DIR:-$HERE/../dist}"
[ -d "$DIST" ] || exit 0
DIST="$(cd "$DIST" && pwd)"

case "$(uname -s 2>/dev/null)" in
  MINGW* | MSYS* | CYGWIN*) ;;
  *) [ -n "${PRUNE_PROCESS_LIST:-}" ] || exit 0 ;;
esac

# The folders to consider. find, not ls: `ls -d` prints a trailing slash on
# these shells and an exact-name exclusion then silently matches nothing.
CANDIDATES="$(find "$DIST" -mindepth 1 -maxdepth 1 -type d -name 'cef-dev-*' 2>/dev/null | sort)"
[ -n "$CANDIDATES" ] || exit 0

deadline=$((SECONDS + WAIT))
removed=0
left=0
while :; do
  PROCS="$(process_paths)"
  if [ -z "$PROCS" ]; then
    echo "prune-dev-builds: couldn't list running processes; deleting nothing" >&2
    exit 0
  fi
  pending=""
  while IFS= read -r dir; do
    [ -n "$dir" ] || continue
    name="$(basename "$dir")"
    [ "$name" = "$KEEP" ] && continue
    if in_use "$dir"; then
      pending="$pending$dir"$'\n'
      continue
    fi
    if [ "$DRY" = 1 ]; then
      echo "prune-dev-builds: would delete $name"
    else
      rm -rf -- "$dir" 2>/dev/null
      if [ -d "$dir" ]; then
        # Part of it was locked after all (a process started between the check
        # and the delete). Leave the rest for the next run.
        echo "prune-dev-builds: couldn't fully delete $name; leaving it" >&2
        left=$((left + 1))
        continue
      fi
      echo "prune-dev-builds: deleted $name"
    fi
    removed=$((removed + 1))
  done <<EOF
$CANDIDATES
EOF
  [ -n "$pending" ] || break
  if [ "$SECONDS" -ge "$deadline" ]; then
    left=$((left + $(printf '%s' "$pending" | grep -c .)))
    break
  fi
  CANDIDATES="${pending%$'\n'}"
  sleep 1
done

if [ "$left" -gt 0 ]; then
  echo "prune-dev-builds: $left build(s) still running or locked; they go on the next run"
fi
exit 0
