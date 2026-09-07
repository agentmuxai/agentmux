#!/usr/bin/env bash
# gen-docs-index.sh — regenerate the machine-maintained half of docs/specs/INDEX.md.
#
# docs/specs/PLAN_DOCS_CLEANUP_EXECUTION_2026_09_01.md (Batch D)
# docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md (Phase 3)
#
# WHY GENERATED, AND WHY GROUPED BY STATUS
#
# The curated index above the marker is genuinely useful and stays hand-written:
# it answers "where do I start for subsystem X", which no generator can.
#
# What it cannot do is keep up. Measured 2026-09-01: it lists 77 of 730 specs,
# and 165 specs added in the previous 30 days are absent from it. Nothing was
# broken — every curated entry still resolves — it is simply being outrun at
# ~165 new docs/month. That is the failure Phase 3 named: "the tool that's
# supposed to help an agent find the current doc is itself an instance of the
# problem it's meant to solve."
#
# Grouping by Status rather than subsystem is deliberate. Subsystem grouping
# needs human judgement (the 308 distinct filename prefixes do not map to
# subsystems), so a generator doing it would produce noise. Status answers the
# question a reader actually arrives with — "is this real, or someone's idea?" —
# and it is now an enforced closed enum (scripts/check-doc-status.sh), so it can
# be trusted enough to sort on.
#
# Usage:
#   bash scripts/gen-docs-index.sh          # rewrite the generated section
#   bash scripts/gen-docs-index.sh --check      # CI: assert, but only when
#                                              # this branch touches docs/specs
#   bash scripts/gen-docs-index.sh --check-all  # assert against the whole tree
#
# MERGE CONFLICT IN INDEX.md? Do not hand-resolve it.
#
#   git checkout --ours docs/specs/INDEX.md && bash scripts/gen-docs-index.sh
#
# Either side is equally wrong once both branches have added specs, and the
# generated content is a pure function of the tree — so regenerating after
# taking either side is always correct, and hand-merging never is. This is a
# committed generated file, so any two PRs that add a spec conflict here; the
# conflict is expected and costs one command.
#
# Two failure modes worth knowing, both hit while building this:
#
#   - CI tests the PR MERGE commit, not your branch head. If --check passes
#     locally and fails on CI, your branch is stale: git will happily merge a
#     new spec's ROW in from main while keeping YOUR section header count, so
#     the merged file is internally inconsistent in a way neither parent was.
#     Merge main, regenerate. The "N specs" line printed on every run is the
#     fastest way to spot it — a count differing from yours means exactly this.
#   - Verifying with a `(?<!...)` lookbehind silently finds nothing: ripgrep's
#     default engine rejects lookaround, and a redirected stderr turns that
#     parse error into a confident "0 matches".

set -uo pipefail

# Byte-identical output on every machine, or --check is a coin flip: it compares
# a committed file against a fresh run, so ANY environment-dependent ordering
# makes CI disagree with the developer who just regenerated. Pathname expansion
# and sort(1) both use LC_COLLATE, which differs between a bare Windows shell
# (effectively C) and a CI runner (C.UTF-8/en_US.UTF-8).
export LC_ALL=C

INDEX="docs/specs/INDEX.md"
BEGIN="<!-- BEGIN GENERATED INDEX — edit scripts/gen-docs-index.sh, not this section -->"
END="<!-- END GENERATED INDEX -->"

check_only=0
check_scope="changed"
case "${1:-}" in
    --check)     check_only=1 ;;
    # Assert against the whole tree regardless of what this branch changed.
    # For local/manual verification and for anyone auditing the index itself;
    # CI deliberately does NOT use it (see below).
    --check-all) check_only=1; check_scope="all" ;;
esac

# ── Why --check is scoped to branches that touch docs/specs ─────────────────
#
# INDEX.md is generated but committed, so it is a shared mutable file that
# every spec-touching PR writes. An unscoped assertion made ANY staleness on
# main fail the gate on EVERY open PR, including PRs that touch no docs at
# all — the PR that caused it is never the PR that fails (issue #3059: five
# regeneration PRs in one session, each blocking unrelated work).
#
# Scoping it makes the failure attributable: a branch that adds a spec or
# changes a Status is the one that must regenerate, and it is the only one
# asked to. A branch that touches no specs cannot make the index stale, so it
# is not held responsible for someone else's omission. This matches what the
# two sibling doc gates already do — `check-doc-status.sh` and
# `check-spec-citations.sh` are both changed-files-only; the index check was
# the odd one out.
#
# Scoping alone would only stop the SPREAD, not the drift — that is issue
# #3059's option 3, which its author correctly called incomplete. The other
# half is already done: #3056 removed the section-header counts, which were
# the only part of the file whose correctness depended on what OTHER branches
# did (two branches each adding a spec merged cleanly into an arithmetically
# wrong total that neither computed). Rows are per-line and merge correctly,
# so concurrent regenerations now compose.
#
# What #3056 cannot cover, and this does: a branch that adds a spec and never
# regenerates at all. The row is simply missing, no arithmetic involved, and
# under an unscoped check that lands on whoever opens a PR next. Scoped, it
# lands on the branch that omitted it. The two together close the class.
should_check() {
    [ "$check_scope" = "all" ] && return 0
    base="${GITHUB_BASE_REF:-main}"
    if git rev-parse --verify --quiet "origin/$base" >/dev/null 2>&1; then
        ref="origin/$base"
    elif git rev-parse --verify --quiet "$base" >/dev/null 2>&1; then
        ref="$base"
    else
        # Same posture as check-doc-status.sh: an unresolvable base is a CI
        # environment problem, not a docs problem. Skip rather than fail a
        # PR for it.
        echo "gen-docs-index: cannot resolve base ref '$base' — skipping."
        return 1
    fi
    # --name-status, not --name-only, and BOTH sides of a rename (Codex P2 on
    # PR #3069). --name-only reports only a rename's DESTINATION, so moving a
    # spec out of docs/specs/ (say, reclassifying it under docs/reports/)
    # showed up as a non-spec path, skipped the check, and left the index
    # carrying a row for a file no longer there. Verified against git 2.55:
    # for a spec moved out of the tree, `--name-only --find-renames` prints
    # the destination path alone, while `--name-status` prints an `R100` row
    # carrying the old path and the new one. (Spelled out rather than shown
    # as a literal example path — check-spec-citations.sh reads any
    # spec-shaped path in a comment as a citation, and a made-up one is a
    # dangling citation by its definition. It caught exactly that here.)
    #
    # Deliberately UNLIKE check-doc-status.sh, which skips pure renames (R100)
    # because relocating a file makes no claim about its Status. The opposite
    # is true here: a pure rename is exactly the kind of change that alters
    # the index, since the row is keyed on the filename.
    #
    # The generator itself is in scope too. A change to it can alter the
    # generated output with no spec touched at all — this very PR changes it —
    # and without this the committed INDEX.md would drift silently until some
    # later spec PR failed for it, which is the unattributable failure this
    # whole change exists to remove.
    #
    # INDEX.md itself counts as well: a hand-edit to it must still be caught.
    if git diff --name-status --find-renames "$ref"...HEAD 2>/dev/null \
        | awk '{ for (i = 2; i <= NF; i++) print $i }' \
        | grep -qE '^(docs/specs/.*[.]md|scripts/gen-docs-index\.sh)$'; then
        return 0
    fi
    echo "gen-docs-index: no specs changed on this branch — index not asserted."
    echo "  (run 'bash scripts/gen-docs-index.sh --check-all' to assert anyway)"
    return 1
}

# Nothing to assert on this branch — exit before doing the work.
if [ "$check_only" -eq 1 ] && ! should_check; then
    exit 0
fi

# ── Build the generated section ─────────────────────────────────────────────
#
# Single pass over the tree. An earlier version looped the whole directory once
# per status word — 730 files x 8 statuses, ~5,800 process spawns — which took
# over two minutes on Windows. Now each file is read once into a status->rows
# map, which is ~730 spawns and seconds.
build() {
    declare -A ROWS
    local total=0 shown=0
    for f in docs/specs/*.md; do
        b=$(basename "$f")
        case "$b" in INDEX.md|README.md) continue ;; esac
        total=$((total + 1))

        # One read per file; pull the Status word and the H1 together.
        # Deliberately no `|| continue` here: an unreadable file must still get
        # a row (it falls through to __none__ with the filename as its title)
        # rather than vanishing. Every path out of this loop ends in a row, so
        # the completeness assertion at the bottom of build() is meaningful.
        head -40 "$f" > "$scratch" 2>/dev/null || :
        w=$(grep -m1 -i '^\*\*Status:\*\*' "$scratch" 2>/dev/null             | sed 's/^\*\*Status:\*\*[[:space:]]*//I'             | awk '{print tolower($1)}' | sed 's/[^a-z]//g')
        title=$(grep -m1 '^# ' "$scratch" 2>/dev/null | sed 's/^# *//' | tr -d '|')
        [ -z "$title" ] && title="$b"
        [ -z "$w" ] && w="__none__"

        ROWS["$w"]="${ROWS[$w]:-}| [\`${b%.md}\`](${b}) | ${title} |
"
    done

    printf '%s
' "$BEGIN"
    printf '
## All specs by status

'
    printf 'Generated by `scripts/gen-docs-index.sh` — do not hand-edit. Covers
'
    printf 'every spec directly in `docs/specs/`, which the curated sections
'
    printf 'above deliberately do not.
'
    printf '
`archive/` is excluded on purpose — it means "not worth reading unless you
'
    printf 'are doing history". Everything else is here; the completeness
'
    printf 'assertion in the generator fails the build rather than emit a
'
    printf 'partial list.
'

    for st in implemented active proposed draft living historical superseded; do
        rows="${ROWS[$st]:-}"
        [ -z "$rows" ] && continue
        n=$(printf '%s' "$rows" | grep -c '^|')
        shown=$((shown + n))
        printf '
### %s

' "$st"
        printf '| Spec | Title |
|---|---|
'
        printf '%s' "$rows"
    done

    # Specs with no Status line at all — surfaced, not dropped. Hiding them
    # would make the index look complete while omitting a sixth of the tree.
    rows="${ROWS[__none__]:-}"
    if [ -n "$rows" ]; then
        n=$(printf '%s' "$rows" | grep -c '^|')
        shown=$((shown + n))
        printf '
### no status line

'
        printf 'Predate the closed vocabulary. Not a backlog to bulk-restamp —
'
        printf 'an unverified restamp turns "unknown" into "confidently wrong".
'
        printf 'Fix one when you touch it and know its real state.

'
        printf '| Spec | Title |
|---|---|
'
        printf '%s' "$rows"
    fi

    # Status line present, but its first word is outside the closed enum.
    #
    # An earlier version of this script printed ONLY the seven canonical
    # buckets, so every one of these landed in a ROWS key the print loop never
    # iterated and vanished from the index with no trace — 189 specs (26% of
    # the tree) silently absent, while the header above claimed to cover every
    # file. That is the exact failure this batch exists to fix, reintroduced by
    # the fix itself. --check could not catch it: it only diffs a re-run of the
    # same logic, so a stable bug stays green forever.
    noncanon=""
    for st in $(printf '%s\n' "${!ROWS[@]}" | sort); do
        case " implemented active proposed draft living historical superseded __none__ " in
            *" $st "*) continue ;;
        esac
        noncanon="$noncanon $st"
    done

    if [ -n "$noncanon" ]; then
        n=0
        for st in $noncanon; do
            n=$((n + $(printf '%s' "${ROWS[$st]}" | grep -c '^|')))
        done
        shown=$((shown + n))
        printf '
### non-canonical status
'
        printf '
These carry a `**Status:**` line whose first word is not in the closed enum
'
        printf '(`docs/specs/README.md`). Grouped by the word actually found, so the
'
        printf 'real state is visible rather than guessed at. `check-doc-status.sh`
'
        printf 'requires a fix the next time one of these is edited; as with the
'
        printf 'section above, do not bulk-restamp them.
'
        for st in $noncanon; do
            printf '
**`%s`**

' "$st"
            printf '| Spec | Title |
|---|---|
'
            printf '%s' "${ROWS[$st]}"
        done
    fi

    # Completeness assertion — the check that would have caught the bug above,
    # and the reason it cannot come back. Every candidate file must be
    # accounted for by exactly one emitted row; if not, the index would be
    # claiming a coverage it does not have, so refuse to produce it at all.
    # Always report the file count, not only on failure. A count that differs
    # between two machines is the first thing worth knowing when --check
    # disagrees with a local run, and inferring it from summed section headers
    # is exactly the guesswork this line removes.
    echo "gen-docs-index: $total specs under docs/specs/ (excluding archive/)" >&2

    if [ "$shown" -ne "$total" ]; then
        echo "gen-docs-index: FATAL — emitted $shown rows for $total specs;" >&2
        echo "  $((total - shown)) unaccounted for. The index would be incomplete" >&2
        echo "  while claiming to cover every file; refusing to write it." >&2
        return 3
    fi

    printf '
%s
' "$END"
}

[ -f "$INDEX" ] || { echo "gen-docs-index: $INDEX not found"; exit 1; }

scratch=$(mktemp)
trap 'rm -f "$scratch"' EXIT

# ── Splice into the index, preserving everything above the marker ───────────
tmp=$(mktemp)
if grep -qF "$BEGIN" "$INDEX"; then
    sed "/$(printf '%s' "$BEGIN" | sed 's/[]\/$*.^[]/\\&/g')/,\$d" "$INDEX" > "$tmp"
else
    cat "$INDEX" > "$tmp"
    printf '\n---\n\n' >> "$tmp"
fi
if ! build >> "$tmp"; then
    # The completeness assertion tripped. Leave INDEX.md exactly as it was:
    # a stale index is recoverable, a confidently-incomplete one is not.
    rm -f "$tmp"
    exit 3
fi

if [ "$check_only" -eq 1 ]; then
    if diff -q "$INDEX" "$tmp" >/dev/null 2>&1; then
        rm -f "$tmp"
        echo "gen-docs-index: INDEX.md is current."
        exit 0
    fi
    echo "gen-docs-index: INDEX.md is STALE."
    echo "  A spec was added, removed, or had its Status changed without"
    echo "  regenerating the index. Run: bash scripts/gen-docs-index.sh"
    echo ""
    # Show WHAT differs, not just that something does. A gate that only says
    # "stale" makes the reader re-derive the diff by hand — and when it fails
    # on CI but not locally, that guesswork is the whole cost of the failure.
    echo "  Difference (committed < vs regenerated >), first 40 lines:"
    diff "$INDEX" "$tmp" 2>&1 | head -40 | sed 's/^/    /'
    rm -f "$tmp"
    exit 1
fi

mv "$tmp" "$INDEX"
echo "gen-docs-index: regenerated $INDEX"
