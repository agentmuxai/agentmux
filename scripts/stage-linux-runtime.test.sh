#!/usr/bin/env bash
# stage-linux-runtime.test.sh — tests for the source-map policy in
# scripts/stage-linux-runtime.sh.
#
# Regression coverage for a real defect: the map-strip block ran
# UNCONDITIONALLY on Linux, contradicting its own comment and
# SPEC_PORTABLE_SOURCE_MAPS_2026_06_01.md ("`task package` (local portable) —
# maps included"). Windows honours `${STRIP_MAPS:-0}`; Linux ignored the
# variable entirely, so every local Linux portable shipped without maps.
#
# That breaks frontend/log/source-map-resolver.ts, which vite.config.ts emits
# maps for on purpose — a packaged build could only report
# `index-<hash>.js (190)` plus a 404. It cost hours on the vim/DECRQM freeze.
#
# Usage: bash scripts/stage-linux-runtime.test.sh   (exit 0 = all pass)

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/stage-linux-runtime.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

# Extract the real block so the test cannot drift from the implementation.
BLOCK="$TMP/strip-block.sh"
awk '/^if \[ "\$\{STRIP_MAPS:-0\}" = "1" \]; then$/,/^fi$/' "$SCRIPT" > "$BLOCK"
if [ ! -s "$BLOCK" ]; then
    bad "extract the strip block" "not found — did the guard change shape?"
else
    run_with() { # $1 = STRIP_MAPS value ("" = unset). Prints surviving map count.
        local stage="$TMP/run-$RANDOM-$RANDOM"
        # The script globs "$STAGING_ROOT/usr/bin/frontend", so STAGING_ROOT is
        # the directory CONTAINING usr/ — an earlier revision of this test was
        # one level off, so the script found nothing, deleted nothing, and the
        # keep-cases passed for the wrong reason.
        mkdir -p "$stage/usr/bin/frontend/assets"
        : > "$stage/usr/bin/frontend/assets/index-abc.js"
        : > "$stage/usr/bin/frontend/assets/index-abc.js.map"
        (
            STAGING_ROOT="$stage"
            if [ -n "$1" ]; then export STRIP_MAPS="$1"; else unset STRIP_MAPS; fi
            # shellcheck disable=SC1090
            . "$BLOCK"
        ) > /dev/null 2>&1
        find "$stage/usr/bin/frontend" -name '*.map' | wc -l
    }

    # Guard the harness itself: with the block removed entirely the map must
    # survive, proving the fixture is wired up and the counts mean something.
    _sanity_stage="$TMP/sanity"
    mkdir -p "$_sanity_stage/usr/bin/frontend/assets"
    : > "$_sanity_stage/usr/bin/frontend/assets/x.js.map"
    if [ "$(find "$_sanity_stage/usr/bin/frontend" -name '*.map' | wc -l)" = "1" ]; then
        ok "harness fixture is wired up (map is findable before any run)"
    else
        bad "harness fixture is wired up" "fixture path is wrong; other results are meaningless"
    fi

    # The default is what local `task package` gets.
    if [ "$(run_with "")" = "1" ]; then
        ok "a local build KEEPS source maps (STRIP_MAPS unset)"
    else
        bad "a local build KEEPS source maps (STRIP_MAPS unset)" \
            "maps were deleted — source-map-resolver.ts will have nothing to read"
    fi

    if [ "$(run_with 0)" = "1" ]; then
        ok "STRIP_MAPS=0 keeps maps"
    else
        bad "STRIP_MAPS=0 keeps maps" "maps deleted despite explicit 0"
    fi

    # Releases must still strip: ~28MB per artifact.
    if [ "$(run_with 1)" = "0" ]; then
        ok "STRIP_MAPS=1 strips maps (release path)"
    else
        bad "STRIP_MAPS=1 strips maps (release path)" "maps survived; release would grow ~28MB"
    fi
fi

# Every Linux release task must opt in, or releases silently start shipping maps.
TASKFILE="$HERE/../Taskfile.yml"
_rel=$(grep -c "RELEASE_CHANNEL=stable bash scripts/package-linux.sh" "$TASKFILE")
_stripped=$(grep -c "STRIP_MAPS=1 RELEASE_CHANNEL=stable bash scripts/package-linux.sh" "$TASKFILE")
if [ "$_rel" -gt 0 ] && [ "$_rel" -eq "$_stripped" ]; then
    ok "all $_rel Linux release task(s) set STRIP_MAPS=1"
else
    bad "all Linux release tasks set STRIP_MAPS=1" "$_stripped of $_rel opt in"
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
