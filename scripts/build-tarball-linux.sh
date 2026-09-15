#!/usr/bin/env bash
# Build AgentMux as a portable .tar.gz for Linux x86_64 — extract and run,
# no install step, no fpm/appimagetool dependency. Portable-archive
# counterpart to Windows' portable ZIP (`task package`) and waveterm's `.zip`.
#
# Part of Phase 1, docs/specs/SPEC_LINUX_DISTRO_TARGETS_AND_DOWNLOADS_PAGE_2026_09_15.md.
#
# Usage:
#   bash scripts/build-tarball-linux.sh [output-dir]
#
#   output-dir defaults to ~/Desktop. The archive is named
#   AgentMux_{version}_amd64-portable.tar.gz in that directory and extracts
#   to a single top-level AgentMux/ directory.
#
# Prerequisites (one-time):
#   - Same build artifacts stage-linux-runtime.sh requires (dist/, target/release/).
#   - No fpm, no appimagetool — this format needs no packaging tool at all.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(node -p "require('./package.json').version")"
OUTDIR="${1:-$HOME/Desktop}"
STAGEDIR="$REPO_ROOT/build/AgentMux-portable"
# Same local-build-label convention as build-appimage-linux.sh — see that
# script's comment for the full rationale.
if [ -n "${AGENTMUX_BUILD_LABEL:-}" ]; then
    OUTPUT="$OUTDIR/AgentMux_${AGENTMUX_BUILD_LABEL}_amd64-portable.tar.gz"
else
    OUTPUT="$OUTDIR/AgentMux_${VERSION}_amd64-portable.tar.gz"
fi

echo "Building AgentMux v$VERSION portable tarball → $OUTPUT"

# --- 1. Stage the shared runtime under AgentMux/ (the archive's single
#        top-level directory, so extracting never scatters files into the
#        user's current directory). ---
rm -rf "$STAGEDIR"
mkdir -p "$STAGEDIR/AgentMux"
bash "$REPO_ROOT/scripts/stage-linux-runtime.sh" "$STAGEDIR/AgentMux"

# --- 2. Top-level launch script — no AppImage runtime, no install step,
#        no wrapper indirection through /usr/bin like the .deb's. Just exec
#        the launcher relative to wherever this got extracted.
#        LD_LIBRARY_PATH: agentmux-cef is built without RPATH (same as the
#        AppImage's — see scripts/linux-apprun.sh's identical comment), so
#        without this the launcher's exec fails to find libcef.so.
#        Codex P1, PR #3236. ---
cat > "$STAGEDIR/AgentMux/agentmux.sh" <<'LAUNCH'
#!/usr/bin/env bash
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export LD_LIBRARY_PATH="$DIR/usr/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$DIR/usr/bin/agentmux-launcher" "$@"
LAUNCH
chmod +x "$STAGEDIR/AgentMux/agentmux.sh"

# --- 3. Archive ---
mkdir -p "$OUTDIR"
rm -f "$OUTPUT"
tar czf "$OUTPUT" -C "$STAGEDIR" AgentMux

echo ""
echo "✓ Built portable tarball: $OUTPUT"
ls -lh "$OUTPUT"
