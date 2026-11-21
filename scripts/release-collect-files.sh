#!/usr/bin/env bash
# Collect the release's package files from the downloaded artifacts, each one only from the artifact that makes it.
#
# Usage:  release-collect-files.sh <artifacts directory> <output directory>
#
# `actions/download-artifact` with no name puts every artifact of the run under <artifacts directory>/<artifact name>/.
# The artifacts come from jobs that ran the built commit's code, so a job could upload one of its own with a package's
# name in it. A package file is therefore taken from its own artifact and from no other, and one found anywhere else
# stops the release:
#   windows-portable          *.7z
#   windows-installer         *.exe
#   windows-msix              *.msix
#   windows-frontend-maps     *.tar.xz
#   linux-appimage            *.AppImage
#   linux-deb                 *.deb
#   linux-rpm                 *.rpm
#   linux-tarball             *.tar.xz
#   server-package-linux-*    agentmux-headless-*.tar.gz
#   server-package-macos-*    agentmux-headless-*.tar.gz
#   server-package-windows-*  agentmux-headless-*-windows-x64.zip
# The macOS DMG is not handled here (release-core.yml takes it from `macos-dmg` and checks its digest).
#
# Exit: 0 collected; 1 a package file in an artifact that does not make it (each is named), or bad arguments.
set -euo pipefail

SRC="${1:-}"
OUT="${2:-}"
[ -d "$SRC" ] && [ -n "$OUT" ] || { echo "usage: release-collect-files.sh <artifacts directory> <output directory>" >&2; exit 1; }
mkdir -p "$OUT"

# <artifact directory glob> <file name glob>, one pair per line.
RULES='windows-portable *.7z
windows-installer *.exe
windows-msix *.msix
windows-frontend-maps *.tar.xz
linux-appimage *.AppImage
linux-deb *.deb
linux-rpm *.rpm
linux-tarball *.tar.xz
server-package-linux-* agentmux-headless-*.tar.gz
server-package-macos-* agentmux-headless-*.tar.gz
server-package-windows-* agentmux-headless-*-windows-x64.zip'

# Every name a package file can have, anywhere.
ANY=(-name '*.7z' -o -name '*.exe' -o -name '*.msix' -o -name '*.AppImage' -o -name '*.deb' -o -name '*.rpm' -o -name '*.tar.xz'
     -o -name 'agentmux-headless-*.tar.gz' -o -name 'agentmux-headless-*-windows-x64.zip')

TAKEN="$(mktemp "${TMPDIR:-/tmp}/agentmux-collect.XXXXXX")"
trap 'rm -f "$TAKEN"' EXIT

while read -r dir name; do
    [ -n "$dir" ] || continue
    # The artifact's own folder, directly under SRC; the file anywhere inside it.
    while IFS= read -r -d '' f; do
        cp "$f" "$OUT/"
        printf '%s\0' "$f" >> "$TAKEN"
    done < <(find "$SRC" -mindepth 2 -type f -path "$SRC/$dir/*" -name "$name" -print0)
done <<< "$RULES"

bad=0
while IFS= read -r -d '' f; do
    if ! grep -qzxF -- "$f" "$TAKEN"; then
        echo "::error::a release package file came in an artifact that does not make it: ${f#"$SRC"/}" >&2
        bad=1
    fi
done < <(find "$SRC" -type f \( "${ANY[@]}" \) -print0)
[ "$bad" = 0 ] || exit 1
echo "✓ collected $(find "$OUT" -type f | wc -l | tr -d ' ') package files from their own artifacts"
