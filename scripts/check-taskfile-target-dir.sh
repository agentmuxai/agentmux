#!/usr/bin/env bash
# check-taskfile-target-dir.sh — CI grep gate: Taskfile.yml never hard-codes cargo's output folder.
#
# THE RULE: a step that reads what cargo built takes the folder from `{{.TARGET_DIR}}` (CARGO_TARGET_DIR when set, else
# `target`), never from a literal `target/release` or `target/debug`. A Windows machine with little room on the system drive
# keeps CARGO_TARGET_DIR on another drive; cargo then builds there, and a step that copies from the literal path fails after
# a build of 30+ minutes (#48305: `task build:backend && task build:host`).
#
# Comment lines are ignored (they explain the rule and name the packagers). The release packagers (scripts/package-portable.sh,
# package-macos.sh, stage-linux-runtime.sh) read `target/` on purpose and are not looked at here: release builds use the
# default target dir, and scripts/check-bundled-tools.sh matches their literal path.
#
# Usage:
#   bash scripts/check-taskfile-target-dir.sh [Taskfile.yml]
# Exit 0 = clean, exit 1 = a literal path (or a TARGET_DIR without toSlash) was found.

set -euo pipefail

FILE="${1:-Taskfile.yml}"
if [ ! -f "$FILE" ]; then
    echo "check-taskfile-target-dir: $FILE does not exist." >&2
    exit 1
fi

fail=0

# 1. No literal cargo output folder outside comment lines.
literal="$(grep -nE 'target[/\\](release|debug|fast-release)' "$FILE" | grep -vE '^[0-9]+:[[:space:]]*#' || true)"
if [ -n "$literal" ]; then
    echo "check-taskfile-target-dir: $FILE hard-codes cargo's output folder (use {{.TARGET_DIR}}/release/...):" >&2
    printf '%s\n' "$literal" | cut -c1-200 >&2
    fail=1
fi

# 2. The variable itself must write the path with forward slashes, so one value is right in PowerShell and in bash.
if ! grep -qE "^[[:space:]]*TARGET_DIR:.*CARGO_TARGET_DIR \| toSlash" "$FILE"; then
    echo "check-taskfile-target-dir: the TARGET_DIR variable in $FILE must pass CARGO_TARGET_DIR through toSlash." >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "check-taskfile-target-dir: $FILE reads cargo's output only through TARGET_DIR"
