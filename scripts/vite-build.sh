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

exec npx vite build "$@"
