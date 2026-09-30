#!/usr/bin/env bash
# migrate-branch-to-crates.sh — finish rebasing a branch created before the
# crates/ move (SPEC_CRATES_DIRECTORY_REORGANIZATION_2026_09_30.md §5.4).
#
# The six crates moved from agentmux-<name>/ to crates/<name>/. After
#
#     git fetch origin && git rebase origin/main
#
# edits your branch made to existing crate files have already followed the
# rename. Two things have not, and this script fixes both:
#
#   1. Files your branch ADDED under an old agentmux-<name>/ folder. Git has
#      no old file to follow, so they stay put, outside the workspace, and
#      Cargo silently ignores them. This moves each to crates/<name>/.
#   2. Paths your branch wrote into files it changed: "agentmux-<name>/..."
#      becomes "crates/<name>/...", "../agentmux-<name>" inside a crate
#      becomes "../<name>", and a new ts-rs `export_to = "../../frontend/`
#      gains one "../".
#
# It then commits the result as one "chore: migrate branch to crates/"
# commit, and exits non-zero if anything is still left under an old folder.
# On a branch with nothing to fix it changes nothing, so it's safe to run
# everywhere.
#
# Usage (from the repo root, after the rebase, with a clean tree):
#     bash scripts/migrate-branch-to-crates.sh [base]    # base defaults to origin/main
#     bash scripts/migrate-branch-to-crates.sh --dry-run [base]
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

DRY_RUN=0
if [[ "${1:-}" == "--dry-run" ]]; then DRY_RUN=1; shift; fi
BASE="${1:-origin/main}"
CRATES="cef|srv|launcher|mcp|common|bashwrap"

if ! git cat-file -e "$BASE:crates/srv/Cargo.toml" 2>/dev/null; then
    echo "migrate-branch-to-crates: $BASE has no crates/ folder yet. Fetch and rebase onto a main that includes the move first." >&2
    exit 2
fi
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    echo "migrate-branch-to-crates: the working tree has uncommitted changes. Commit or stash them first." >&2
    exit 2
fi

run() { if (( DRY_RUN )); then echo "  would run: $*"; else "$@"; fi; }

# 1. Files the branch added under an old folder.
moved=0
while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    name="${f#agentmux-}"; name="${name%%/*}"
    dest="crates/${name}/${f#agentmux-${name}/}"
    echo "move: $f -> $dest"
    run mkdir -p "$(dirname "$dest")"
    run git mv "$f" "$dest"
    moved=$((moved + 1))
done < <(git diff --name-only --diff-filter=A "$BASE"...HEAD | grep -E "^agentmux-($CRATES)/" || true)

# 2. Old paths inside files the branch changed (after the moves above).
rewritten=0
while IFS= read -r f; do
    [[ -z "$f" || ! -f "$f" ]] && continue
    grep -Iq . "$f" 2>/dev/null || continue   # skip binary files
    case "$f" in docs/specs/*|docs/retro/*|docs/reports/*|docs/analysis/*|VERSION_HISTORY.md|.changesets/*) continue ;; esac
    before=$(cksum < "$f")
    if (( DRY_RUN )); then
        if grep -qE "(^|[^A-Za-z0-9_-])agentmux-($CRATES)/|export_to = \"\.\./\.\./frontend/" "$f"; then
            echo "  would rewrite paths in: $f"
        fi
        continue
    fi
    if [[ "$f" == crates/* ]]; then
        sed -i -E "s#\.\./agentmux-($CRATES)([/\"'\\\\])#../\1\2#g" "$f"
        sed -i -E 's#export_to = "\.\./\.\./frontend/#export_to = "../../../frontend/#g' "$f"
    fi
    sed -i -E "s#(^|[^A-Za-z0-9_-])agentmux-($CRATES)/#\1crates/\2/#g" "$f"
    if [[ "$(cksum < "$f")" != "$before" ]]; then
        echo "rewrite: $f"
        git add -- "$f"
        rewritten=$((rewritten + 1))
    fi
done < <(git diff --name-only --diff-filter=AMR "$BASE"...HEAD; git diff --name-only --cached)

if (( DRY_RUN )); then
    echo "migrate-branch-to-crates: dry run, nothing changed."
    exit 0
fi

left=$(git ls-files | grep -E "^agentmux-($CRATES)/" || true)
if [[ -n "$left" ]]; then
    echo "migrate-branch-to-crates: files are still under an old folder:" >&2
    echo "$left" >&2
    exit 1
fi
if (( moved + rewritten == 0 )); then
    echo "migrate-branch-to-crates: nothing to migrate."
    exit 0
fi
git commit -q -m "chore: migrate branch to crates/ (moved $moved file(s), rewrote paths in $rewritten)"
echo "migrate-branch-to-crates: committed. Build and test before pushing."
