#!/usr/bin/env bash
# Wrapper for vite build that ensures the CWD uses the true filesystem casing.
# On Windows, Git Bash lowercases the drive letter (c:\systems vs C:\Systems),
# which breaks Vite's html-inline-proxy and vite-tsconfig-paths plugins due to
# case-sensitive path comparisons in Rollup.

set -euo pipefail

# Get the real (native-cased) path of the project root
REAL_ROOT=$(node -e "console.log(require('fs').realpathSync.native(process.cwd()))")

# cd to the native-cased path so process.cwd() matches filesystem casing
cd "$REAL_ROOT"

# Node's default V8 old-space limit (~2GB) is no longer enough headroom for
# this frontend's production build — it started crashing with "JavaScript
# heap out of memory" (exit 134) on the macOS CI runner, reproducing on both
# ci-nightly-artifacts.yml and release.yml. CI runners across all three
# platforms have well over 4GB free, Node just wasn't told it could use it.
export NODE_OPTIONS="${NODE_OPTIONS:-} --max-old-space-size=4096"

exec npx vite build "$@"
