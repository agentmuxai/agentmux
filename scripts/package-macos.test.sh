#!/usr/bin/env bash
# package-macos.test.sh — tests for the source-map policy in
# scripts/package-macos.sh.
#
# Regression coverage for a real defect: the map-strip block defaulted to
# STRIP_MAPS=1 (strip) on macOS — the opposite of every other platform, and
# of docs/specs/SPEC_PORTABLE_SOURCE_MAPS_2026_06_01.md's own contract
# ("`task package` (local portable) — maps included"). Windows honours
# `${STRIP_MAPS:-0}` in scripts/package.sh; Linux was fixed to match in
# scripts/stage-linux-runtime.sh (#3355, see stage-linux-runtime.test.sh).
# macOS was the last platform out of line, so every local `task package:macos`
# build shipped without maps.
#
# That breaks frontend/log/source-map-resolver.ts, which vite.config.ts emits
# maps for on purpose — a packaged crash could only report
# `index-<hash>.js (190)` plus a 404. It cost hours on the vim/DECRQM freeze
# (docs/retro/retro-xterm-requestmode-minify-freeze-2026-09-17.md).
#
# Usage: bash scripts/package-macos.test.sh   (exit 0 = all pass)

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/package-macos.sh"
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
        local app="$TMP/run-$RANDOM-$RANDOM/AgentMux.app"
        mkdir -p "$app/Contents/Resources/frontend/assets"
        : > "$app/Contents/Resources/frontend/assets/index-abc.js"
        : > "$app/Contents/Resources/frontend/assets/index-abc.js.map"
        (
            APP="$app"
            if [ -n "$1" ]; then export STRIP_MAPS="$1"; else unset STRIP_MAPS; fi
            # shellcheck disable=SC1090
            . "$BLOCK"
        ) > /dev/null 2>&1
        # `wc -l` alone is not portable here: BSD wc (macOS, where this
        # script actually runs) right-pads its count with leading spaces
        # ("       1"), unlike GNU wc — a bare string comparison against
        # "1"/"0" would silently fail on every count, on the one platform
        # this test exists to cover. `tr -d ' '` normalizes both.
        find "$app/Contents/Resources/frontend" -name '*.map' | wc -l | tr -d ' '
    }

    # Guard the harness itself: with the block removed entirely the map must
    # survive, proving the fixture is wired up and the counts mean something.
    _sanity_app="$TMP/sanity/AgentMux.app"
    mkdir -p "$_sanity_app/Contents/Resources/frontend/assets"
    : > "$_sanity_app/Contents/Resources/frontend/assets/x.js.map"
    if [ "$(find "$_sanity_app/Contents/Resources/frontend" -name '*.map' | wc -l | tr -d ' ')" = "1" ]; then
        ok "harness fixture is wired up (map is findable before any run)"
    else
        bad "harness fixture is wired up" "fixture path is wrong; other results are meaningless"
    fi

    # The default is what local `task package:macos` gets.
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

# The macOS release task must opt in, or releases silently start shipping
# maps — the exact regression that motivated this fix (package:release:macos
# never set STRIP_MAPS=1 at all, unlike Windows's package:release and every
# package:release:linux* variant).
TASKFILE="$HERE/../Taskfile.yml"
if grep -q "STRIP_MAPS=1 RELEASE_CHANNEL=stable bash scripts/package-macos.sh" "$TASKFILE"; then
    ok "package:release:macos sets STRIP_MAPS=1"
else
    bad "package:release:macos sets STRIP_MAPS=1" "release DMGs would ship with source maps"
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
