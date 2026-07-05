#!/usr/bin/env bash
# Verify the release-consistency invariant from CLAUDE.md:
#
#   In every commit, these MUST all equal the same version:
#     - VERSION_HISTORY.md's top `## X.Y.Z` section
#     - package.json.version
#     - Cargo.toml [workspace.package].version
#     - Cargo.lock's workspace-member versions (e.g. agentmux-cef)
#     - package-lock.json's root version
#
# Exit codes:
#   0 — all five locations agree
#   1 — at least one location disagrees (prints the mismatched set)
#   2 — internal error reading a location
#
# Usage:
#   scripts/verify-release-consistency.sh          # check current working tree
#   scripts/verify-release-consistency.sh --quiet  # only print on failure
#
# This script is invoked by:
#   - scripts/release.sh as the final gate after `task release`
#   - .github/workflows/release-consistency.yml on PRs/pushes that touch
#     VERSION_HISTORY.md (release-intent commits)
#
# Why we don't check on every package.json bump: between releases,
# `task package` patch-bumps package.json + Cargo.toml + lockfiles while
# VERSION_HISTORY.md stays pinned to the last shipped release — that's the
# operational design. The five-way agreement is only required when a
# release-intent commit lands (signaled by VH being touched).
#
# History: docs/retro/retro-release-version-desync-2026-05-22.md.
# v0.38.0 shipped with package.json stranded at 0.37.2 because bump-cli
# silently skipped the file. The reagent gate caught it on a later PR,
# the release-script self-verify catches it on `task release`, this
# CI workflow catches any other path (manual edit, rebase artifact, etc).

set -euo pipefail

QUIET=0
if [[ "${1:-}" == "--quiet" ]]; then
    QUIET=1
fi

cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

log() { (( QUIET )) && return; echo "$*"; }

# ── Read every location ───────────────────────────────────────────────
PKG_V="$(node -p "require('./package.json').version" 2>/dev/null || echo '<unreadable>')"
CARGO_V="$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\(.*\)"$/\1/p;}' Cargo.toml | head -1)"
PL_V="$(node -p "require('./package-lock.json').version" 2>/dev/null || echo '<unreadable>')"
CL_V="$(sed -n '/^name = "agentmux-cef"$/{n;s/^version = "\(.*\)"$/\1/p;q}' Cargo.lock)"
VH_V="$(awk '/^## /{print $2; exit}' VERSION_HISTORY.md)"

declare -A LOCATIONS=(
    [package.json]="$PKG_V"
    [Cargo.toml]="$CARGO_V"
    [package-lock.json]="$PL_V"
    [Cargo.lock]="$CL_V"
    [VERSION_HISTORY.md]="$VH_V"
)

# ── Compute the consensus (most-common value) ────────────────────────
#
# We compare every location against the modal value. If only one place
# disagrees, that's the bad file. If they all disagree, the report
# tells the operator which set they need to reconcile.
declare -A COUNTS=()
for loc in "${!LOCATIONS[@]}"; do
    v="${LOCATIONS[$loc]}"
    COUNTS[$v]=$((${COUNTS[$v]:-0} + 1))
done

CONSENSUS=""
CONSENSUS_COUNT=0
for v in "${!COUNTS[@]}"; do
    if (( COUNTS[$v] > CONSENSUS_COUNT )); then
        CONSENSUS="$v"
        CONSENSUS_COUNT="${COUNTS[$v]}"
    fi
done

# ── Report ───────────────────────────────────────────────────────────
MISMATCHES=()
for loc in package.json Cargo.toml package-lock.json Cargo.lock VERSION_HISTORY.md; do
    v="${LOCATIONS[$loc]}"
    if [[ "$v" != "$CONSENSUS" ]]; then
        MISMATCHES+=("$loc: $v (expected $CONSENSUS)")
    fi
done

if (( ${#MISMATCHES[@]} == 0 )); then
    log "Release-consistency OK — all 5 locations agree on version $CONSENSUS."
    exit 0
fi

# Print failures even in --quiet mode; this is the actionable signal.
echo "" >&2
echo "ERROR: release-consistency check failed." >&2
echo "Most files report version '$CONSENSUS', but these disagree:" >&2
for m in "${MISMATCHES[@]}"; do
    echo "  - $m" >&2
done
echo "" >&2
echo "All five locations must equal the same version. This usually means" >&2
echo "bump-cli silently skipped a file, or a rebase resurrected a stale" >&2
echo "version line. Inspect:" >&2
echo "  git diff HEAD~1 -- package.json Cargo.toml Cargo.lock package-lock.json VERSION_HISTORY.md" >&2
echo "Then fix the mismatched files." >&2
echo "" >&2
echo "See docs/retro/retro-release-version-desync-2026-05-22.md." >&2
exit 1
