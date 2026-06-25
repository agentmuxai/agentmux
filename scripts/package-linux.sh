#!/usr/bin/env bash
# Orchestrate a LOCAL portable AppImage build with an ephemeral, traceable
# label and a per-build data-dir channel — mirrors scripts/package.sh (Windows).
#
# This does NOT bump the version and does NOT touch git. The committed
# release version moves ONLY through `task release` (changesets). A local
# build is *labeled*, not *versioned* — see
# docs/specs/SPEC_LOCAL_BUILD_VERSIONING_2026_05_28.md.
#
# Usage:
#   bash scripts/package-linux.sh [--fresh] [output-dir]
#
#   --fresh      No-op (accepted for back-compat). Every local build is now
#                already its own isolated data dir — see CHANNEL below — so
#                there is nothing left for --fresh to do.
#   output-dir   Where the AppImage lands (default ~/Desktop).
#
# What gets stamped:
#   - VERSION  : the semver core from package.json, UNCHANGED.
#   - LABEL    : <version>+g<sha>[.dirty].<stamp>.<pid> — semver build metadata.
#                Names the output file so builds are unique on disk.
#   - CHANNEL  : local-<slug>-<branch-hash>-<build-id> — the data-dir key.
#                PER-BUILD: each build is its own AgentMux instance (unique
#                data dir + cef-cache + single-instance pipe). Baked at compile
#                time via AGENTMUX_BUILD_CHANNEL_DEFAULT. Safe because agents
#                and auth are global (#1387-#1393); only pane layout + memories
#                start fresh. Releases use RELEASE_CHANNEL=stable.
#
# Release / CI path:
#   RELEASE_CHANNEL=stable bash scripts/package-linux.sh [output-dir]
#   (task package:release:linux sets this automatically)

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

DIRTY=""
if ! git diff --quiet 2>/dev/null || ! git diff --cached --quiet 2>/dev/null; then
    DIRTY=".dirty"
fi

STAMP=$(date -u +%Y%m%dT%H%M%S)
LABEL="${VERSION}+g${SHA}${DIRTY}.${STAMP}.$$"

# Channel = data-dir key. Same scheme as scripts/package.sh (Windows):
#   "local-" (6) + slug (≤27) + "-" + hash (6) + "-" + build-id (8) = ≤55 chars
# well under the 64-char cap in data_paths.rs::sanitize_channel_name.
BRANCH_HASH=$(printf '%s' "$BRANCH" | sha1sum | cut -c1-6)
BRANCH_SLUG=$(printf '%s' "$BRANCH" | tr -c 'A-Za-z0-9._-' '-' | cut -c1-27)
BUILD_ID=$(printf '%s' "$LABEL" | sha1sum | cut -c1-8)
CHANNEL="local-${BRANCH_SLUG}-${BRANCH_HASH}-${BUILD_ID}"

if [ -n "${RELEASE_CHANNEL:-}" ]; then
    if [ "$FRESH" -eq 1 ]; then
        echo "ERROR: --fresh and RELEASE_CHANNEL are mutually exclusive." \
             "A release AppImage always uses a fixed channel ('$RELEASE_CHANNEL');" \
             "a fresh throwaway dir doesn't make sense for a distributed artifact." >&2
        exit 1
    fi
    CHANNEL="$RELEASE_CHANNEL"
fi

echo "────────────────── linux appimage build ──────────────────"
echo "  version : $VERSION   (unchanged — no bump, no git mutation)"
echo "  label   : $LABEL"
echo "  channel : $CHANNEL"
if [ -n "${RELEASE_CHANNEL:-}" ]; then
    echo "  data    : RELEASE — channel override: $RELEASE_CHANNEL"
else
    echo "  data    : per-build — isolated dir (agents + auth carry over globally; pane layout + memories start fresh)"
fi
echo "──────────────────────────────────────────────────────────"

# NOTE: per-build channels accumulate on disk. Cleanup belongs in launcher
# startup (it can check pipe liveness); `rm -rf` here risks corrupting a live
# instance. Until then, prune ~/.agentmux/channels/local-* manually.

# Export BEFORE cargo builds — agentmux-common/build.rs reads this via
# option_env! and bakes it in; it declares rerun-if-env-changed so a changed
# channel forces a recompile rather than serving a stale cache.
export AGENTMUX_BUILD_CHANNEL_DEFAULT="$CHANNEL"
# Only export the label for local builds. Release builds use CARGO_PKG_VERSION
# as the pipe key so same-version stable instances continue to share the
# single-instance guard and data dir as designed.
if [ -z "${RELEASE_CHANNEL:-}" ]; then
    export AGENTMUX_BUILD_LABEL="$LABEL"
fi

task build:frontend
task build:backend
task build:host
task copy:schema
task bundle

if [ -n "$OUTDIR" ]; then
    bash scripts/build-appimage-linux.sh "$OUTDIR"
else
    bash scripts/build-appimage-linux.sh
fi
