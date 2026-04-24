#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Repair a partial `cef_windows_x86_64/` extract after download-cef's
# `fs::rename` chain races Windows Defender. See
# `docs/retros/RETRO_CEF_BUILD_RACE_2026_04_24.md` for the full root
# cause. This script is idempotent — safe to call when the extract is
# already complete (it's a no-op then).
#
# Exit codes:
#   0 — extract is complete (either was already, or we repaired it)
#   1 — no cef-dll-sys build dir found (extraction hasn't started yet)
#   2 — repair needed but source dir to copy from is missing/empty

set -euo pipefail

ROOT="${CARGO_TARGET_DIR:-target}/release/build"
# Find the latest cef-dll-sys build dir that has an `out/` subdir.
# Multiple hashes can coexist; we want the one that actually did work.
DEST_DIR=""
for d in "$ROOT"/cef-dll-sys-*/out/cef_windows_x86_64; do
    if [ -d "$d" ]; then
        DEST_DIR="$d"
        break
    fi
done

if [ -z "$DEST_DIR" ]; then
    # No cef_windows_x86_64 yet — extraction hasn't run at all, nothing to repair.
    exit 1
fi

# Sibling raw-extract dir (source of truth for headers/source)
SRC_PARENT="$(dirname "$DEST_DIR")"
SRC_DIR=""
for d in "$SRC_PARENT"/cef_binary_*/; do
    if [ -d "$d" ]; then
        SRC_DIR="$d"
        break
    fi
done

repaired=0
for sub in libcef_dll include Resources; do
    dest="$DEST_DIR/$sub"
    # "Present but empty" counts as missing for Resources (the rename loop
    # drained its contents into DEST_DIR but Resources/ itself is left behind).
    if [ ! -e "$dest" ] || { [ -d "$dest" ] && [ -z "$(ls -A "$dest" 2>/dev/null)" ]; }; then
        if [ -z "$SRC_DIR" ]; then
            echo "repair-cef-extract: $dest missing and no cef_binary_*/ source to copy from" >&2
            exit 2
        fi
        src="$SRC_DIR$sub"
        if [ ! -d "$src" ]; then
            # Resources was drained — nothing to copy; that's fine.
            [ "$sub" = "Resources" ] && continue
            echo "repair-cef-extract: source $src missing" >&2
            exit 2
        fi
        rm -rf "$dest"
        cp -r "$src" "$dest"
        echo "repair-cef-extract: restored $dest"
        repaired=1
    fi
done

if [ "$repaired" = "1" ]; then
    echo "repair-cef-extract: CEF extract repaired; re-running build should succeed"
fi
exit 0
