#!/usr/bin/env bash
# Configure the patched-CEF build tree with AgentMux's canonical GN args so a
# from-scratch rebuild produces the slim, official-build libcef.so by default.
#
# Without this, the size-reduction flags (is_official_build=true, ...) lived only
# in an untracked out/Release_GN_x64/args.gn on one machine — wipe ~/cef-build and
# a rebuild silently regresses to the ~2x-larger non-official binary. This script
# makes scripts/cef-build/args.gn (version-controlled) the source of truth.
#
# Usage:
#   bash scripts/cef-build/configure-cef-build.sh
#   AGENTMUX_CEF_SRC=/path/to/chromium/src bash scripts/cef-build/configure-cef-build.sh
#
# Prereqs: the chromium+cef tree must already be synced and patched (see
# docs/cef-build/build-patched-libcef.md steps 1-3). This script only does the
# `gn gen` configure step (step 4) — it does NOT sync, patch, or build.
#
# What it does (idempotent):
#   1. Regenerate the gitignored CEF C-API wrappers via translator.py — these get
#      cleaned and the build otherwise dies instantly on
#      `cef/libcef_dll/ctocpp/views/window_ctocpp.cc` "missing and no known rule"
#      (it's translator-generated for the patched CefWindow::begin_window_drag API).
#   2. Copy the canonical args.gn into out/Release_GN_x64/ (backing up any existing).
#   3. Run `gn gen out/Release_GN_x64`.
#
# After it succeeds, build with the OOM-resistant ninja wrapper (step 5 of the doc).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CANONICAL_ARGS="$SCRIPT_DIR/args.gn"
CEF_SRC="${AGENTMUX_CEF_SRC:-$HOME/cef-build/chromium_git/chromium/src}"
OUT_DIR="out/Release_GN_x64"

if [ ! -f "$CANONICAL_ARGS" ]; then
    echo "ERROR: canonical args not found at $CANONICAL_ARGS" >&2
    exit 1
fi
if [ ! -d "$CEF_SRC" ]; then
    echo "ERROR: chromium/src tree not found at $CEF_SRC" >&2
    echo "       Sync + patch it first (docs/cef-build/build-patched-libcef.md steps 1-3)," >&2
    echo "       or set AGENTMUX_CEF_SRC to your chromium/src path." >&2
    exit 1
fi
if [ ! -f "$CEF_SRC/cef/tools/translator.py" ]; then
    echo "ERROR: $CEF_SRC/cef/tools/translator.py missing — is the cef checkout in place?" >&2
    exit 1
fi

cd "$CEF_SRC"

# 1. Regenerate the translator-produced C-API wrappers (gitignored; cleaned by
#    `git clean` / fresh checkouts). NO `--quiet` — it's an invalid option.
echo "==> Regenerating CEF C-API wrappers (translator.py)…"
( cd cef && python3 tools/translator.py --root-dir . )

# 2. Install the canonical args.gn (back up any existing one).
mkdir -p "$OUT_DIR"
if [ -f "$OUT_DIR/args.gn" ] && ! cmp -s "$CANONICAL_ARGS" "$OUT_DIR/args.gn"; then
    backup="$OUT_DIR/args.gn.bak-$(node -p 'Date.now()' 2>/dev/null || echo prev)"
    cp "$OUT_DIR/args.gn" "$backup"
    echo "==> Backed up existing args.gn → $backup"
fi
cp "$CANONICAL_ARGS" "$OUT_DIR/args.gn"
echo "==> Installed canonical args.gn → $CEF_SRC/$OUT_DIR/args.gn"

# 3. gn gen — prefer the in-tree gn (no PATH dependency), fall back to PATH.
if [ -x "buildtools/linux64/gn" ]; then
    GN="./buildtools/linux64/gn"
elif command -v gn >/dev/null 2>&1; then
    GN="gn"
else
    echo "ERROR: no gn binary — expected buildtools/linux64/gn or gn on PATH" >&2
    echo "       (add depot_tools to PATH, or run \`gclient sync\` to populate buildtools)." >&2
    exit 1
fi
echo "==> Running $GN gen $OUT_DIR …"
"$GN" gen "$OUT_DIR"

echo ""
echo "✓ Configured $CEF_SRC/$OUT_DIR with the canonical official-build args."
echo "  Next: build with the OOM-resistant wrapper (doc step 5):"
echo "    systemd-run --user --scope --collect --unit=cef-build.scope ~/cef-build/ninja-with-retry.sh"
