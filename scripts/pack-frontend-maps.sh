#!/usr/bin/env bash
# Pack a frontend build's source maps into one archive, for a release whose installer leaves them out.
#
#   scripts/pack-frontend-maps.sh <platform> <version> <out dir> [frontend dir]
#
# The archive is <out dir>/AgentMux_<version>_<platform>-frontend-maps.tar.xz, with the maps at the paths they have in
# the frontend dir (assets/<chunk>.js.map), so `node scripts/source-maps.mjs resolve <unpacked dir> <stack file>`
# decodes a stack from that release. Every platform builds its own frontend, so each one that strips its maps packs
# its own: a chunk's hash can differ between them.
#
# It checks the build's maps first (scripts/source-maps.mjs check) and fails if any are missing or hold a build
# machine's path, so a release never ships an archive with holes, and never ships one with a runner's file paths.
set -euo pipefail

if [ "$#" -lt 3 ]; then
    echo "usage: $0 <platform> <version> <out dir> [frontend dir]" >&2
    exit 2
fi
PLATFORM="$1"
VERSION="$2"
OUT_DIR="$3"
FRONTEND="${4:-dist/frontend}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

node "$SCRIPT_DIR/source-maps.mjs" check "$FRONTEND"

mkdir -p "$OUT_DIR"
ARCHIVE="$(cd "$OUT_DIR" && pwd)/AgentMux_${VERSION}_${PLATFORM}-frontend-maps.tar.xz"
LIST="$(mktemp)"
trap 'rm -f "$LIST"' EXIT
(cd "$FRONTEND" && find . -name '*.map' | LC_ALL=C sort | sed 's#^\./##') > "$LIST"
COUNT="$(wc -l < "$LIST" | tr -d ' ')"
if [ "$COUNT" -eq 0 ]; then
    echo "ERROR: no source maps under $FRONTEND to pack." >&2
    exit 1
fi
(cd "$FRONTEND" && tar -cJf "$ARCHIVE" -T "$LIST")
echo "  maps    : packed $COUNT source maps into $(basename "$ARCHIVE") ($(du -h "$ARCHIVE" | cut -f1))"
