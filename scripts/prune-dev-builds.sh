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

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIST="${PRUNE_DIST_DIR:-$HERE/../dist}"
[ -d "$DIST" ] || exit 0
DIST="$(cd "$DIST" && pwd)"

case "$(uname -s 2>/dev/null)" in
  MINGW* | MSYS* | CYGWIN*) ;;
  *) [ -n "${PRUNE_PROCESS_LIST:-}" ] || exit 0 ;;
esac

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

# Windows form of a folder with a trailing backslash, lowercased, so a prefix
# match can't confuse cef-dev-17 with cef-dev-170.
win_prefix() {
  local p="$1"
  if command -v cygpath >/dev/null 2>&1; then p="$(cygpath -w "$p")"; fi
  printf '%s\\' "$(printf '%s' "$p" | tr 'A-Z' 'a-z')"
}

in_use() { # <folder>; reads $PROCS
  local prefix
  prefix="$(win_prefix "$1")"
  # A bash string match, not grep: Git for Windows' grep aborts (SIGABRT) on a
  # fixed-string pattern ending in a backslash, and as a pipeline reader its
  # early exit would trip pipefail. Both sides are already lowercased.
  case "$PROCS" in
    *"$prefix"*) return 0 ;;
    *) return 1 ;;
  esac
}

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
