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
#   - CHANNEL  : local-<branch>[-<stamp> if --fresh] — the data-dir key.
#                Branch-scoped so rebuilds of the same branch reuse one
#                data dir. Baked into the binary at compile time via
#                AGENTMUX_BUILD_CHANNEL_DEFAULT (see agentmux-common/build.rs).
#   - AGENTMUX_BUILD_LABEL: the full label (including stamp) baked into
#                the launcher for use as the single-instance pipe key, so
#                two successive local builds never share a pipe and each
#                local build starts its own window.

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
#   "local-" (6) + slug (≤27) + "-" + hash (6) + "-" + build-id (8)
#   = ≤55  → comfortably under 64.
BRANCH_HASH=$(printf '%s' "$BRANCH" | sha1sum | cut -c1-6)
BRANCH_SLUG=$(printf '%s' "$BRANCH" | tr -c 'A-Za-z0-9._-' '-' | cut -c1-27)
# Per-BUILD isolation (one AgentMux instance per build). The channel is the
# data-dir + cef-cache + single-instance-pipe key, so making it unique per build
# means a freshly-built binary launches as its OWN instance instead of joining a
# still-running sibling build of the same branch. (#1315 fixed only the pipe; the
# cef-cache stayed per-branch — data_paths.rs:209 derives it from the channel — so
# a second build's host hit Chromium's user-data-dir singleton and exited with
# "Opening in existing browser session". Stamping the channel fixes all three keys
# at once.) Safe to isolate per build now that agents + auth are GLOBAL
# (cross-channel work #1387-#1393): a fresh per-build data dir still sees every
# agent and stays logged in; only pane layout + memories start fresh. BUILD_ID
# hashes the full LABEL (sha+dirty+stamp+pid) so even concurrent same-second builds
# of one branch get distinct channels, well within the 64-char cap.
BUILD_ID=$(printf '%s' "$LABEL" | sha1sum | cut -c1-8)
CHANNEL="local-${BRANCH_SLUG}-${BRANCH_HASH}-${BUILD_ID}"
# --fresh is redundant now (every local build is already its own data dir); kept
# as an accepted no-op so existing invocations / muscle memory don't break.

# Release portables must bake the "stable" channel so users open their
# real data dir, not a branch-scoped local dir. Set RELEASE_CHANNEL
# (task package:release does this automatically) to override.
if [ -n "${RELEASE_CHANNEL:-}" ]; then
    if [ "$FRESH" -eq 1 ]; then
        echo "ERROR: --fresh and RELEASE_CHANNEL are mutually exclusive." \
             "A release portable always uses a fixed channel ('$RELEASE_CHANNEL');" \
             "a fresh throwaway dir doesn't make sense for a distributed artifact." >&2
        exit 1
    fi
    CHANNEL="$RELEASE_CHANNEL"
fi

echo "────────────────────── local build ──────────────────────"
echo "  version : $VERSION   (unchanged — no bump, no git mutation)"
echo "  label   : $LABEL"
echo "  channel : $CHANNEL"
if [ -n "${RELEASE_CHANNEL:-}" ]; then
    echo "  data    : RELEASE — channel override: $RELEASE_CHANNEL"
else
    echo "  data    : per-build — isolated dir (agents + auth carry over globally; pane layout + memories start fresh)"
fi
echo "──────────────────────────────────────────────────────────"

# GC: per-build channels accumulate (each build leaves a data dir + cef-cache,
# tens–hundreds of MB). For LOCAL builds, keep the few most-recent per-build
# channels for THIS branch and prune older ones so an iterate-rebuild loop doesn't
# grow the disk without bound. Skips anything modified in the last 30 min (a crude
# "might still be running" guard) so a live sibling build is never nuked. The
# pre-per-build shared channel (`local-<slug>-<hash>` with no build-id suffix) does
# NOT match the glob and is preserved. Best-effort: never fails the build.
if [ -z "${RELEASE_CHANNEL:-}" ]; then
    GC_KEEP=${AGENTMUX_LOCAL_CHANNELS_KEEP:-5}
    CH_ROOT="${HOME}/.agentmux/channels"
    if [ -d "$CH_ROOT" ] && [ -n "$BRANCH_HASH" ]; then
        ls -dt "$CH_ROOT/local-${BRANCH_SLUG}-${BRANCH_HASH}-"* 2>/dev/null \
            | tail -n +$((GC_KEEP + 1)) \
            | while IFS= read -r old; do
                [ -n "$(find "$old" -maxdepth 0 -mmin -30 2>/dev/null)" ] && continue
                rm -rf "$old" 2>/dev/null && echo "  gc      : pruned old build channel $(basename "$old")"
              done
    fi
fi

# Exported for:
#   - the cargo builds: AGENTMUX_BUILD_CHANNEL_DEFAULT is read via
#     option_env! in data_paths.rs and baked in. agentmux-common/build.rs
#     declares rerun-if-env-changed for it so a changed channel actually
#     recompiles instead of serving a stale cache.
#   - package-portable.sh: AGENTMUX_BUILD_LABEL names the artifacts.
# Locally-built portables default to the local channel family; release
# portables use RELEASE_CHANNEL (set by task package:release) to bake
# "stable"; release CI never calls this script.
export AGENTMUX_BUILD_CHANNEL_DEFAULT="$CHANNEL"
# Only export AGENTMUX_BUILD_LABEL for local builds. Release builds (RELEASE_CHANNEL
# set) must use CARGO_PKG_VERSION as their pipe key so same-version stable instances
# continue to share the single-instance guard and data dir as designed.
if [ -z "${RELEASE_CHANNEL:-}" ]; then
    export AGENTMUX_BUILD_LABEL="$LABEL"
fi

task build:frontend

# Release portables strip source maps (~28 MB). Local builds keep them so
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
