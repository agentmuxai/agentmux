#!/usr/bin/env bash
# Stable-channel build guard.
#
# Only a released state of the repo may claim the "stable" channel —
# stable-channel artifacts open the user's REAL data dir, so a build from
# a dirty tree or an unmerged branch would let not-really-released code
# write into it. Enforced mechanically here instead of by convention:
#
#   1. no tracked modifications in the working tree,
#   2. HEAD is a `chore: release v*` commit,
#   3. HEAD is reachable from origin/main (best-effort fetch first).
#
# Escape hatch for emergencies: AGENTMUX_STABLE_OVERRIDE=1.
set -euo pipefail

refuse() {
    echo "[stable-guard] REFUSED: $1" >&2
    echo "[stable-guard] stable-channel artifacts open the user's real data dir;" >&2
    echo "[stable-guard] build from a merged 'chore: release v*' commit with a clean tree," >&2
    echo "[stable-guard] use 'task package' (isolated local-* channel) for anything else," >&2
    echo "[stable-guard] or set AGENTMUX_STABLE_OVERRIDE=1 to bypass deliberately." >&2
    exit 1
}

if [ "${AGENTMUX_STABLE_OVERRIDE:-0}" = "1" ]; then
    echo "[stable-guard] AGENTMUX_STABLE_OVERRIDE=1 — skipping release-state checks" >&2
    exit 0
fi

if [ -n "$(git status --porcelain -uno)" ]; then
    refuse "working tree has tracked modifications"
fi

subject=$(git log -1 --format=%s)
case "$subject" in
    "chore: release v"*) ;;
    *) refuse "HEAD is not a release commit (subject: '$subject')" ;;
esac

# Best-effort ref refresh; offline builds still validate against the
# last-known origin/main rather than failing on the fetch itself.
git fetch origin main -q 2>/dev/null || true
if ! git merge-base --is-ancestor HEAD origin/main 2>/dev/null; then
    refuse "HEAD is not on origin/main (unmerged or unpushed release commit)"
fi

echo "[stable-guard] OK: clean tree at released commit $(git rev-parse --short HEAD) ('$subject')"
