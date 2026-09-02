#!/usr/bin/env bash
# check-doc-status.sh — CI gate for the docs Status vocabulary.
#
# docs/specs/README.md (the closed enum, docs-lifecycle hardening Phase 1)
# docs/specs/PLAN_DOCS_CLEANUP_EXECUTION_2026_09_01.md (Batch E)
#
# THE RULES, both already adopted by the repo and neither previously enforced:
#
#   1. A doc's `**Status:**` line must begin with one of:
#        draft | proposed | active | implemented | living | historical | superseded
#   2. `superseded` REQUIRES a `**Superseded-by:**` line, and that pointer must
#      resolve to a file that exists. The README's words: "a broken pointer is
#      worse than none."
#
# SCOPED TO CHANGED FILES, deliberately. As of 2026-09-01 roughly 318 of 729
# specs predate the vocabulary; a repo-wide gate would fail every PR on
# pre-existing violations and be switched off within a day — which is precisely
# how the previous three attempts at this died. Applied to the diff it is always
# green for compliant work and stops the backlog growing, which is the goal.
#
# Rule 1 is skipped for files with NO Status line at all: adding one requires
# knowing what the doc's state actually IS, which is a judgement call, not
# something a gate can demand mid-PR from someone fixing a typo. Rule 2 is
# enforced whenever a Status line says `superseded`, because that is a claim the
# author made in this diff.
#
# Usage:
#   bash scripts/check-doc-status.sh            # changed vs origin/main (or $GITHUB_BASE_REF)
#   bash scripts/check-doc-status.sh FILE...    # explicit files (used by the tests)

set -uo pipefail

VALID="draft proposed active implemented living historical superseded"
fail=0

# ── Which files to check ────────────────────────────────────────────────────
explicit=0
if [ "$#" -gt 0 ]; then
    files=("$@")
    explicit=1
else
    base="${GITHUB_BASE_REF:-main}"
    # In CI the base branch may not be fetched; fall back to whatever we have.
    if git rev-parse --verify --quiet "origin/$base" >/dev/null 2>&1; then
        ref="origin/$base"
    elif git rev-parse --verify --quiet "$base" >/dev/null 2>&1; then
        ref="$base"
    else
        echo "check-doc-status: cannot resolve base ref '$base' — skipping."
        exit 0
    fi
    mapfile -t files < <(
        # A whole directory, not a '**/*.md' glob: that form requires at least
        # one directory level, so a root-level file in the pathspec was never
        # selected — silently skipping part of what the gate was meant to cover.
        # The .md filter happens in the loop instead.
        #
        # `specs` was a second pathspec until the two spec trees were merged
        # into docs/specs/ (batch C); there is no top-level specs/ anymore.
        git diff --name-only --diff-filter=d "$ref"...HEAD -- docs 2>/dev/null | grep -E '[.]md$' || true
    )
fi

# Which changed files are NEW, for the stricter new-doc rule below.
added_files=""
if [ "$explicit" -eq 0 ]; then
    added_files=$(git diff --name-only --diff-filter=A "$ref"...HEAD -- docs 2>/dev/null | grep -E '[.]md$' || true)
fi

# ── One spec tree, not two ──────────────────────────────────────────────────
#
# The top-level specs/ tree was merged into docs/specs/ (batch C). It is worth
# a hard check rather than a convention, because the split did not persist by
# anyone's decision — it persisted because nothing ever objected to it, through
# two audits that recommended merging.
#
# What made it actively harmful: directory-as-lifecycle and the Status: field
# answered the same question differently, and promoting a file between trees
# silently broke every code comment citing it (32 of 165 citations were already
# dangling when the trees were merged).
if [ -d specs ]; then
    echo "FAIL specs/"
    echo "     A top-level specs/ tree exists. Every spec belongs in docs/specs/;"
    echo "     a doc's lifecycle is its **Status:** line, not its directory."
    echo "     See docs/specs/README.md."
    fail=1
fi

[ "${#files[@]}" -eq 0 ] && { echo "check-doc-status: no docs changed."; exit 0; }


# ── Check each ──────────────────────────────────────────────────────────────
checked=0
for f in "${files[@]}"; do
    [ -f "$f" ] || continue
    # The docs/ filter applies only to the git-diff path — an explicit file
    # list means the caller already chose, and silently skipping their files
    # (as an earlier version did for absolute paths) is worse than checking
    # something out of tree.
    if [ "$explicit" -eq 0 ]; then
        case "$f" in
            docs/*) ;;
            *) continue ;;
        esac
    fi
    checked=$((checked + 1))

    status_line=$(grep -m1 -i '^\*\*Status:\*\*' "$f" 2>/dev/null || true)

    if [ -z "$status_line" ]; then
        # A doc ADDED in this diff must carry a Status: you are writing it, so
        # you know its state, and 126 status-less specs already exist because
        # nothing ever asked. A doc that merely CHANGED and never had one is not
        # blocked — demanding a status from someone fixing a typo in a
        # four-month-old file is how a gate gets resented and then disabled.
        if [ "$explicit" -eq 0 ] && printf '%s
' "$added_files" | grep -qxF "$f"; then
            echo "FAIL $f"
            echo "     New doc has no **Status:** line."
            echo "     One of: $VALID  (see docs/specs/README.md)"
            fail=1
        fi
        continue
    fi

    word=$(printf '%s' "$status_line" \
        | sed 's/^\*\*Status:\*\*[[:space:]]*//I' \
        | awk '{print tolower($1)}' \
        | sed 's/[^a-z]//g')

    if ! printf '%s' "$VALID" | grep -qw "$word"; then
        echo "FAIL $f"
        echo "     Status must start with one of: $VALID"
        echo "     got: ${status_line:0:100}"
        echo "     see docs/specs/README.md"
        fail=1
        continue
    fi

    [ "$word" != "superseded" ] && continue

    # ── superseded: pointer required, and it must resolve ───────────────────
    ptr_line=$(grep -m1 -i '^\*\*Superseded-by:\*\*' "$f" 2>/dev/null || true)
    if [ -z "$ptr_line" ]; then
        echo "FAIL $f"
        echo "     Status is 'superseded' but there is no **Superseded-by:** line."
        echo "     A superseded doc must say what replaced it. If nothing did,"
        echo "     the status is wrong — a spec whose design shipped is"
        echo "     'implemented'; one that merely records a past effort is"
        echo "     'historical'. Do not invent a pointer to satisfy this check."
        fail=1
        continue
    fi

    # Extract every `*.md` path token on the line; pass if ANY of them resolves.
    #
    # One grep rather than a pipeline of sed/tr: the README mandates no
    # particular form, so all of these must work —
    #     **Superseded-by:** ./x.md
    #     **Superseded-by:** [`x.md`](./x.md)
    #     **Superseded-by:** See docs/specs/x.md for details.
    # An earlier version took the first whitespace token and reported "See" as a
    # missing file, false-failing a compliant doc — which is how a gate loses
    # trust and gets switched off. A later attempt stitched two extractions
    # together and silently concatenated them into one bogus path, breaking the
    # markdown-link form that is the most common in this repo. Both bugs were
    # the pipeline, not the rule; grep -o for the thing we actually want is
    # simpler and has neither failure mode.
    #
    # Erring toward accepting is correct: the rule is "the pointer must go
    # somewhere real", and one resolving path satisfies it.
    candidates=$(printf '%s\n' "$ptr_line" | grep -oE '[A-Za-z0-9._/-]+\.md' | sort -u)

    # A pointer naming no repo path at all (blank field, or prose with no path)
    # does NOT satisfy the rule: the README requires it "pointing at a real path
    # in this repo". Accepting it would let a bare `**Superseded-by:**` with
    # nothing after it pass — the broken-pointer case wearing a different hat.
    if [ -z "$candidates" ]; then
        echo "FAIL $f"
        echo "     Superseded-by names no repository path."
        echo "     It must point at a real .md file in this repo (docs/specs/README.md)."
        echo "     got: ${ptr_line:0:100}"
        fail=1
        continue
    fi

    resolved=0
    while IFS= read -r target; do
        [ -z "$target" ] && continue
        if [ -e "$(dirname "$f")/$target" ] || [ -e "$target" ]; then
            resolved=1
            break
        fi
    done <<< "$candidates"

    if [ "$resolved" -eq 0 ]; then
        echo "FAIL $f"
        echo "     Superseded-by names no path that exists. Tried:"
        printf '       %s\n' $candidates
        echo "     A broken pointer is worse than none (docs/specs/README.md)."
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    echo ""
    echo "check-doc-status: FAILED"
    exit 1
fi

echo "check-doc-status: ok ($checked doc(s) checked)"
