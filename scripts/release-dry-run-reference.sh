#!/usr/bin/env bash
# FROZEN REFERENCE for scripts/release-dry-run.test.mjs. Do not edit.
#
# This is scripts/release.sh as it was when it started one awk process per
# changeset (two per file), cut off after the `--dry-run` exit: everything the
# dry run prints, nothing that bumps. The test runs it and the real
# scripts/release.sh over the same changesets and requires byte-identical
# output, so the one-pass reader in release.sh provably changed nothing.

set -euo pipefail

DRY_RUN=0
FORCE_TYPE=""
FORCE_SET=0   # whether --as/--bump was passed (so an empty value still errors)
# Each branch consumes its own args — no loop-bottom `shift`, which under
# `set -euo pipefail` would abort cryptically when `--as` is the final token
# (the in-branch shift empties $@, then a second shift fails).
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --as|--bump)
            FORCE_SET=1
            FORCE_TYPE="${2:-}"          # empty if no value follows → caught below
            shift 2 2>/dev/null || shift  # consume flag (+value); tolerate missing value
            ;;
        --as=*|--bump=*)
            FORCE_SET=1
            FORCE_TYPE="${1#*=}"          # empty for `--as=` → caught below
            shift
            ;;
        *)
            echo "ERROR: unknown argument '$1' (expected --dry-run and/or --as <patch|minor|major>)." >&2
            exit 1
            ;;
    esac
done
if (( FORCE_SET )); then
    case "$FORCE_TYPE" in
        patch|minor|major) ;;
        *)
            echo "ERROR: --as must be one of patch|minor|major (got '$FORCE_TYPE')." >&2
            exit 1
            ;;
    esac
fi

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$REPO_ROOT" ]]; then
    echo "ERROR: not inside a git repository." >&2
    exit 1
fi
cd "$REPO_ROOT"

shopt -s nullglob
CHANGESETS=(.changesets/[0-9]*.md)

if (( ${#CHANGESETS[@]} == 0 )); then
    echo "No pending changesets. Nothing to release." >&2
    exit 0
fi

echo "Found ${#CHANGESETS[@]} pending changeset(s):" >&2
for f in "${CHANGESETS[@]}"; do
    echo "  - ${f#.changesets/}" >&2
done
echo >&2

# Determine highest bump type. Order: major > minor > patch.
RELEASE_TYPE="patch"
for f in "${CHANGESETS[@]}"; do
    TYPE="$(awk '/^---$/{n++;next} n==1 && /^type:/{print $2; exit}' "$f" | tr -d '[:space:]')"
    case "$TYPE" in
        major) RELEASE_TYPE="major"; break ;;
        minor) [[ "$RELEASE_TYPE" != "major" ]] && RELEASE_TYPE="minor" ;;
        patch) ;;
        *)
            echo "WARNING: $f has unknown type '$TYPE', treating as patch." >&2
            ;;
    esac
done

# Apply an explicit override (`--as <type>`), if given. The computed type is
# kept so we can warn when the override under-claims (e.g. forcing patch while
# minor changesets are pending). The higher-typed changesets are still consumed
# and changelogged — only the version number is forced lower.
if [[ -n "$FORCE_TYPE" ]]; then
    rank() { case "$1" in patch) echo 0 ;; minor) echo 1 ;; major) echo 2 ;; *) echo 0 ;; esac; }
    if [[ "$FORCE_TYPE" != "$RELEASE_TYPE" ]]; then
        echo "NOTE: --as overrides the computed bump '$RELEASE_TYPE' with '$FORCE_TYPE'." >&2
        if (( $(rank "$FORCE_TYPE") < $(rank "$RELEASE_TYPE") )); then
            echo "WARNING: forcing a LOWER bump than the changesets request — some" >&2
            echo "         ${RELEASE_TYPE}-level change(s) will ship under a $FORCE_TYPE version." >&2
        fi
    fi
    RELEASE_TYPE="$FORCE_TYPE"
fi

echo "Release type: $RELEASE_TYPE" >&2

# Capture one description per changeset: the first non-empty body line (the
# changeset title). A changeset body may carry extra prose lines for reviewer
# context; those must NOT each become their own changelog bullet (issue #1200).
DESCRIPTIONS=()
for f in "${CHANGESETS[@]}"; do
    TITLE="$(awk '/^---$/{n++;next} n==2 && NF {print; exit}' "$f")"
    [[ -n "$TITLE" ]] && DESCRIPTIONS+=("$TITLE")
done

if (( DRY_RUN )); then
    echo >&2
    echo "DRY RUN — would bump $RELEASE_TYPE with these entries:" >&2
    for d in "${DESCRIPTIONS[@]}"; do
        echo "  - $d" >&2
    done
    exit 0
fi
