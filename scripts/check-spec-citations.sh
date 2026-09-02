#!/usr/bin/env bash
# check-spec-citations.sh — CI gate: a cited spec path must resolve.
#
# docs/specs/PLAN_DOCS_CLEANUP_EXECUTION_2026_09_01.md §7 (why this exists)
# docs/specs/README.md ("a broken pointer is worse than none")
#
# THE RULE: if a file names `docs/specs/<something>.md`, that file must exist.
#
# This repo cites specs from code comments constantly, which is a good habit —
# it is often the only link between a design and its implementation. It is also
# unmaintained: when the top-level specs/ tree was merged into docs/specs/
# (batch C, PR #2920), 32 of 165 spec citations in source were ALREADY dangling,
# 9 of them pointing at a path for a file that had moved months earlier. Nothing
# ever checked, so nobody knew, and each one sends the next reader hunting for a
# document that is not there.
#
# SCOPED TO CHANGED FILES, for the same reason check-doc-status.sh is. Measured
# 2026-09-01 with this script over every tracked .md/.rs/.ts/.tsx/.sh/.yml:
#
#     90 dangling citations, across 78 files, naming 49 distinct missing specs
#
# A repo-wide gate would fail every PR on somebody else's rot and be switched
# off within a day — the documented fate of the three previous attempts at docs
# enforcement here. Applied to the diff it is green for anyone who has not
# touched a citation, and it stops 90 becoming 91.
#
# Those three numbers count different things and are easy to conflate: one file
# can carry several dangling citations, and one missing spec can be cited from
# many files. Quote the one you mean.
#
# Usage:
#   bash scripts/check-spec-citations.sh          # changed vs origin/main
#   bash scripts/check-spec-citations.sh FILE...  # explicit files (tests)

set -uo pipefail

fail=0

# ── Which files to check ────────────────────────────────────────────────────
if [ "$#" -gt 0 ]; then
    files=("$@")
else
    base="${GITHUB_BASE_REF:-main}"
    if git rev-parse --verify --quiet "origin/$base" >/dev/null 2>&1; then
        ref="origin/$base"
    elif git rev-parse --verify --quiet "$base" >/dev/null 2>&1; then
        ref="$base"
    else
        echo "check-spec-citations: cannot resolve base ref '$base' — skipping."
        exit 0
    fi
    # No pathspec: rename detection needs both sides of a move to pair them,
    # and limiting the diff hides the deletions (the bug that made batch C's
    # rename exemption a silent no-op). Pure renames are skipped for the same
    # reason as in check-doc-status.sh — relocating a file asserts nothing.
    mapfile -t files < <(
        git diff --name-status --find-renames --diff-filter=d "$ref"...HEAD 2>/dev/null | awk '
            $1 == "R100" { next }
            $1 ~ /^R/    { print $3; next }
            NF >= 2      { print $2 }
        ' || true
    )
fi

if [ "${#files[@]}" -eq 0 ]; then
    echo "check-spec-citations: nothing changed."
    exit 0
fi

# ── Check each ──────────────────────────────────────────────────────────────
checked=0
bad=0
for f in "${files[@]}"; do
    [ -f "$f" ] || continue
    case "$f" in
        # Binary-ish and vendored trees have no citations worth reading, and
        # scanning them is how a gate becomes slow enough to be resented.
        *.png|*.jpg|*.jpeg|*.gif|*.ico|*.svg|*.pdf|*.zip|*.lock) continue ;;
        node_modules/*|target/*|dist/*) continue ;;
    esac
    checked=$((checked + 1))

    # A citation is a docs/specs path ending in .md. Globs are excluded: this
    # file and its sibling gate both discuss `docs/specs/*.md` as a pattern,
    # and failing on a pattern that was never meant to name one file is exactly
    # the false positive that gets a gate disabled.
    while IFS= read -r cite; do
        [ -z "$cite" ] && continue
        case "$cite" in *[\*\?\[]*) continue ;; esac
        [ -e "$cite" ] && continue
        echo "FAIL $f"
        echo "     Cites a spec that does not exist: $cite"
        bad=$((bad + 1))
        fail=1
    done < <(grep -oE 'docs/specs/[A-Za-z0-9_./-]+\.md' "$f" 2>/dev/null | sort -u)
done

if [ "$fail" -ne 0 ]; then
    echo ""
    echo "check-spec-citations: FAILED — $bad dangling citation(s)."
    echo ""
    echo "  If you MOVED a spec, repoint the citations to where it now lives."
    echo "  If the spec never existed or is long gone, delete the path from the"
    echo "  comment rather than leaving it: a pointer to nothing costs the next"
    echo "  reader a search that cannot succeed, which is worse than no pointer"
    echo "  at all (docs/specs/README.md). Do not invent a plausible-looking"
    echo "  replacement path to satisfy this check."
    exit 1
fi

echo "check-spec-citations: ok ($checked file(s) checked)"
