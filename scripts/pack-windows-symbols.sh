#!/usr/bin/env bash
# Pack the Windows release's debug symbols (PDBs) into one archive that stays OUT of the release.
#
#   scripts/pack-windows-symbols.sh <version> <out dir> [cargo target dir]
#
# A srv that wedges is dumped by the launcher (a minidump of thread stacks), and a dump is only readable
# by function name with the PDB of the exact build that wrote it. The build always makes the PDBs; this keeps
# them, one archive per version, at <out dir>/symbols/AgentMux_<version>_windows-symbols.tar.xz.
#
# The archive is deliberately not beside the installers: it is in a sub-folder that no upload step globs, so it
# is never a release asset. Where it is stored afterwards is the release workflow's decision, and it is private.
#
# srv's PDB is required (a release that cannot name a wedged srv's frames fails here, not at the next wedge). The
# launcher's and the host's are packed when the build made them.
set -euo pipefail

if [ "$#" -lt 2 ]; then
    echo "usage: $0 <version> <out dir> [cargo target dir]" >&2
    exit 2
fi
VERSION="$1"
OUT_DIR="$2"
TARGET_DIR="${3:-${CARGO_TARGET_DIR:-target}}/release"

# The names cargo gives a binary's PDB: the binary's name with its dashes as underscores.
REQUIRED="agentmux_srv.pdb"
OPTIONAL="agentmux_launcher.pdb agentmux_cef.pdb"

FILES=()
for name in $REQUIRED; do
    if [ ! -s "$TARGET_DIR/$name" ]; then
        echo "ERROR: $TARGET_DIR/$name is missing or empty: a wedged srv could not be read by function name." >&2
        exit 1
    fi
    FILES+=("$name")
done
for name in $OPTIONAL; do
    if [ -s "$TARGET_DIR/$name" ]; then
        FILES+=("$name")
    else
        echo "  symbols : $name not built, left out"
    fi
done

SYMBOLS_DIR="$OUT_DIR/symbols"
mkdir -p "$SYMBOLS_DIR"
ARCHIVE="$(cd "$SYMBOLS_DIR" && pwd)/AgentMux_${VERSION}_windows-symbols.tar.xz"
rm -f "$ARCHIVE"
# --verbatim-files-from is not needed: the names above are fixed, not read from a build.
(cd "$TARGET_DIR" && tar -cJf "$ARCHIVE" "${FILES[@]}")
echo "  symbols : packed ${FILES[*]} into symbols/$(basename "$ARCHIVE") ($(du -h "$ARCHIVE" | cut -f1)); not a release asset"
