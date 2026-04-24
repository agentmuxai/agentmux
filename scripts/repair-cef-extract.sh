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

# Repair every cef-dll-sys build dir that has a `cef_windows_x86_64/`
# extract — multiple hashes can coexist (e.g. build-script-only vs.
# the actual compiled crate), and a lexicographic-first match may not
# be the hash cargo's current build is watching. Touching all of them
# is idempotent: complete extracts are left alone, partial ones get
# the missing subdirs restored. Codex flagged this in #543 review.
any_candidates=0
any_repaired=0
global_rc=1  # 1 = no candidates found; flipped to 0 if we visit any.

for DEST_DIR in "$ROOT"/cef-dll-sys-*/out/cef_windows_x86_64; do
    if [ ! -d "$DEST_DIR" ]; then
        continue
    fi
    any_candidates=1
    global_rc=0

    # Sibling raw-extract dir (source of truth for headers/source).
    SRC_PARENT="$(dirname "$DEST_DIR")"
    SRC_DIR=""
    for cand in "$SRC_PARENT"/cef_binary_*/; do
        if [ -d "$cand" ]; then
            SRC_DIR="$cand"
            break
        fi
    done

    # --- libcef_dll and include: directory-level renames ------------
    # download-cef moves each of these as a whole directory into
    # cef_windows_x86_64/. When the rename failed the dest is absent;
    # restore it by copying the whole source directory.
    for sub in libcef_dll include; do
        dest="$DEST_DIR/$sub"
        if [ ! -e "$dest" ] || { [ -d "$dest" ] && [ -z "$(ls -A "$dest" 2>/dev/null)" ]; }; then
            if [ -z "$SRC_DIR" ]; then
                echo "repair-cef-extract: $dest missing and no cef_binary_*/ source" >&2
                exit 2
            fi
            src="$SRC_DIR$sub"
            if [ ! -d "$src" ]; then
                echo "repair-cef-extract: source $src missing" >&2
                exit 2
            fi
            rm -rf "$dest"
            cp -r "$src" "$dest"
            echo "repair-cef-extract: restored $dest"
            any_repaired=1
        fi
    done

    # --- Resources: content-level merge, NOT a nested dir -----------
    # download-cef's `for entry in fs::read_dir(&resources)?` loop
    # moves each entry from cef_binary_*/Resources/ INTO the root of
    # cef_windows_x86_64/ (chrome_100_percent.pak, icudtl.dat,
    # locales/, …). A naive `cp -r Resources DEST/` would instead
    # create a nested cef_windows_x86_64/Resources/ that
    # bundle:windows (Taskfile.yml $cefDir/*.pak, $cefDir/locales/*)
    # never reads — Codex P1 on #543. Always merge entries into root;
    # never create a nested Resources/ directory.
    #
    # `icudtl.dat` is the well-known resource we use as the
    # content-present marker. Its absence means the Resources-loop in
    # download-cef never completed for this extract.
    if [ ! -e "$DEST_DIR/icudtl.dat" ]; then
        # Prefer a stray nested Resources/ if a prior (buggy) repair
        # left files there — those entries are canonical because
        # download-cef may have already drained the cef_binary_*/Resources/
        # source during its partial run. Fall back to the source extract.
        merge_src=""
        if [ -d "$DEST_DIR/Resources" ] && [ -n "$(ls -A "$DEST_DIR/Resources" 2>/dev/null)" ]; then
            merge_src="$DEST_DIR/Resources"
        elif [ -n "$SRC_DIR" ] && [ -d "$SRC_DIR/Resources" ]; then
            merge_src="${SRC_DIR}Resources"
        fi
        if [ -n "$merge_src" ]; then
            for entry in "$merge_src"/*; do
                [ -e "$entry" ] || continue  # glob may expand empty
                name="$(basename "$entry")"
                dest="$DEST_DIR/$name"
                if [ ! -e "$dest" ]; then
                    cp -r "$entry" "$dest"
                fi
            done
            echo "repair-cef-extract: merged $merge_src/ into $DEST_DIR root"
            any_repaired=1
        fi
    fi

    # Tidy up any nested Resources/ (stray from a prior buggy repair,
    # or drained-empty from download-cef). Its files are now in root
    # so the dir itself is redundant at best, misleading at worst
    # (the bundler glob would otherwise re-include duplicate .paks).
    if [ -d "$DEST_DIR/Resources" ]; then
        rm -rf "$DEST_DIR/Resources"
    fi
done

if [ "$any_candidates" = "0" ]; then
    # No cef_windows_x86_64 anywhere — extraction hasn't run, nothing
    # for us to do. Signal to the wrapper so it can decide whether to
    # re-run cargo or give up.
    exit 1
fi

if [ "$any_repaired" = "1" ]; then
    echo "repair-cef-extract: CEF extract repaired; re-running build should succeed"
fi
exit 0
