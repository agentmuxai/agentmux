#!/usr/bin/env bash
# Orchestrate a LOCAL portable build with an ephemeral, traceable label.
#
# This does NOT bump the version and does NOT touch git. The committed
# release version moves ONLY through `task release` (changesets). A local
# build is *labeled*, not *versioned* — see
# docs/specs/SPEC_LOCAL_BUILD_VERSIONING_2026_05_28.md.
#
# Usage:
#   bash scripts/package.sh [--fresh] [output-dir]
#
#   --fresh      Give this build a throwaway data dir instead of the
#                branch's persistent one. Normally every local build of a
#                given branch shares one data dir (so your test session —
#                agents, panes, auth — survives an iterate-rebuild loop).
#                --fresh suffixes the channel with the build stamp so this
#                one build starts clean. (Triggers a recompile of the
#                crates that bake the channel — occasional, expected.)
#   output-dir   Where the portable lands (default ~/Desktop).
#
# What gets stamped:
#   - VERSION  : the semver core from package.json, UNCHANGED. Drives the
#                binary filenames + the in-binary version + the
#                package-portable.sh version verification.
#   - LABEL    : <version>+g<sha>[.dirty].<stamp> — semver build metadata
#                (everything after '+' is ignored for precedence, so it can
#                never collide with or reorder a release). Names the
#                portable folder + ZIP so builds are unique on disk and
#                you can tell them apart at a glance.
#   - CHANNEL  : dev-portable-<branch>[-<stamp> if --fresh] — the data-dir
#                key. Branch-scoped so rebuilds of the same branch reuse
#                one data dir. Baked into the binary at compile time via
#                AGENTMUX_BUILD_CHANNEL_DEFAULT (see agentmux-common/build.rs).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

FRESH=0
OUTDIR=""
for arg in "$@"; do
    case "$arg" in
        --fresh) FRESH=1 ;;
        --*) echo "ERROR: unknown flag $arg (supported: --fresh)" >&2; exit 1 ;;
        *) OUTDIR="$arg" ;;
    esac
done

VERSION=$(node -p "require('./package.json').version")
BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "nogit")
SHA=$(git rev-parse --short HEAD 2>/dev/null || echo "nosha")

# Dirty = any unstaged OR staged change in the working tree. Mirrors
# `git describe --dirty`'s notion; flags a build made from uncommitted
# changes so a ZIP labeled `.dirty` is never mistaken for a clean artifact.
DIRTY=""
if ! git diff --quiet 2>/dev/null || ! git diff --cached --quiet 2>/dev/null; then
    DIRTY=".dirty"
fi

# Compact UTC stamp, no dots so it's safe inside a channel name (the
# data-path sanitizer is strict). Always advances → unique folder per
# build, even on a dirty rebuild where the sha doesn't change.
STAMP=$(date -u +%Y%m%dT%H%M%S)

# Append the PID so two package jobs for the same commit started within
# the SAME wall-clock second still produce distinct labels — and thus
# distinct folders/ZIPs — instead of overwriting each other. Concurrent
# builds are the normal multi-agent state this script supports. The PID
# only goes in the LABEL (folder name, unbounded), not the channel, so it
# can't push the channel past its length cap. Codex P2 on #1141.
LABEL="${VERSION}+g${SHA}${DIRTY}.${STAMP}.$$"

# Channel = data-dir key. Coerce anything outside [A-Za-z0-9._-] to '-'
# (git allows '/' in branch names; the data-path sanitizer rejects it).
# A 6-char hash of the FULL branch name keeps distinct long branches that
# share a prefix from aliasing to the same data dir once the human-readable
# slug is truncated. Budget for the 64-char channel cap enforced in
# data_paths.rs::sanitize_channel_name:
#   "dev-portable-" (13) + slug (≤20) + "-" + hash (6) + "-" + stamp (15)
#   = ≤55  → comfortably under 64 even with the --fresh suffix.
# Codex P2 on #1141.
BRANCH_HASH=$(printf '%s' "$BRANCH" | sha1sum | cut -c1-6)
BRANCH_SLUG=$(printf '%s' "$BRANCH" | tr -c 'A-Za-z0-9._-' '-' | cut -c1-20)
CHANNEL="dev-portable-${BRANCH_SLUG}-${BRANCH_HASH}"
if [ "$FRESH" -eq 1 ]; then
    CHANNEL="${CHANNEL}-${STAMP}"
fi

# Release portables must bake the "stable" channel so users open their
# real data dir, not a branch-scoped dev-portable dir. Set RELEASE_CHANNEL
# (task package:release does this automatically) to override.
if [ -n "${RELEASE_CHANNEL:-}" ]; then
    CHANNEL="$RELEASE_CHANNEL"
fi

echo "────────────────────── local build ──────────────────────"
echo "  version : $VERSION   (unchanged — no bump, no git mutation)"
echo "  label   : $LABEL"
echo "  channel : $CHANNEL"
if [ -n "${RELEASE_CHANNEL:-}" ]; then
    echo "  data    : RELEASE — channel override: $RELEASE_CHANNEL"
elif [ "$FRESH" -eq 1 ]; then
    echo "  data    : FRESH — throwaway dir for this build only"
else
    echo "  data    : persistent — shared across rebuilds of '$BRANCH'"
fi
echo "──────────────────────────────────────────────────────────"

# Exported for:
#   - the cargo builds: AGENTMUX_BUILD_CHANNEL_DEFAULT is read via
#     option_env! in data_paths.rs and baked in. agentmux-common/build.rs
#     declares rerun-if-env-changed for it so a changed channel actually
#     recompiles instead of serving a stale cache.
#   - package-portable.sh: AGENTMUX_BUILD_LABEL names the artifacts.
# Locally-built portables default to the dev-portable channel family per
# SPEC_DATA_CHANNELS_2026_05_24.md §2.2; release portables use RELEASE_CHANNEL
# (set by task package:release) to bake "stable"; release CI never calls this script.
export AGENTMUX_BUILD_CHANNEL_DEFAULT="$CHANNEL"
export AGENTMUX_BUILD_LABEL="$LABEL"

task build:frontend

# Release portables strip source maps (~28 MB). Dev portables keep them so
# the runtime source-map resolver works during local testing.
# Set STRIP_MAPS=1 explicitly (task package:release does this automatically).
if [ "${STRIP_MAPS:-0}" = "1" ]; then
    echo "  maps    : stripping .map files (release portable)"
    find dist/frontend -name "*.map" -delete
fi

task build:backend
task build:host
task bundle

if [ -n "$OUTDIR" ]; then
    bash scripts/package-portable.sh "$OUTDIR"
else
    bash scripts/package-portable.sh
fi
