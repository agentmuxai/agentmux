#!/usr/bin/env bash
# check-no-transition-all.sh — CI grep gate: no `transition: all` in the frontend.
#
# docs/analysis/ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md §3
#
# THE RULE: never transition `all`. Window tabs and pane tabs hide inactive
# content with `visibility: hidden`, which every descendant inherits, and
# `visibility` is animatable. An element whose transition covers `all`
# therefore animates each show and hide: on show its first frame samples
# `hidden`, so it pops in a frame after the rest of the tab; on hide it stays
# `visible` for the whole duration. Measured: 56 such transitions per
# window-tab switch, and a late frame on 12 of 12 switches until the rules
# listed their properties instead.
#
# Flags, in frontend/ (tests excluded):
#   - `transition: all ...` / `transition-property: ... all ...` (SCSS/CSS)
#   - an inline style `transition: "all ..."` (TS/TSX)
#   - the Tailwind `transition-all` class
# Fix: list the properties, e.g.
#   transition-property: color, background-color, border-color, outline-color, box-shadow, opacity;
#
# Why a grep gate and not stylelint: stylelint isn't run in CI (the tree has
# hundreds of pre-existing findings), and this one rule must hold now.
#
# Usage:
#   bash scripts/check-no-transition-all.sh
# Exit 0 = clean, exit 1 = a `transition: all` was found.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT"

found="$(
    {
        grep -rnE --include='*.scss' --include='*.css' \
            '(^|[;{[:space:]])transition[[:space:]]*:[[:space:]]*all([[:space:],;]|$)|transition-property[[:space:]]*:[^;]*\ball\b' \
            frontend || true
        grep -rnE --include='*.ts' --include='*.tsx' \
            "transition[\"']?[[:space:]]*:[[:space:]]*[\"'\`]all\b|\btransition-all\b" \
            frontend | grep -vE '\.test\.tsx?:' || true
    } | grep -vE '^\S+:[0-9]+:\s*(//|\*|/\*)' || true
)"

if [[ -n "$found" ]]; then
    echo "ERROR: \`transition: all\` found. It animates the inherited \`visibility\` that hides"
    echo "window and pane tabs, so the element pops in a frame late (see"
    echo "docs/analysis/ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md §3). List the properties instead:"
    echo
    echo "$found"
    exit 1
fi

echo "OK: no \`transition: all\` in frontend/."
