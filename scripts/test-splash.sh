#!/usr/bin/env bash
# test-splash.sh — Run AgentMux via the launcher so the pre-splash is visible.
#
# Usage: bash scripts/test-splash.sh [vite-url]
#   Default URL: http://localhost:5173 (assumes Vite is already running)
#
# Requirements: run `task build:host && task bundle` first.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
URL="${1:-http://localhost:5173}"
TEST_DIR="$REPO_ROOT/dist/splash-test"
RUNTIME_DIR="$TEST_DIR/runtime"
VERSION=$(grep -m1 '"version"' "$REPO_ROOT/package.json" | sed 's/.*"version": *"\([^"]*\)".*/\1/')

echo "==> Setting up splash test dir: $TEST_DIR"

# Clean and recreate
rm -rf "$TEST_DIR"
mkdir -p "$RUNTIME_DIR/locales"

# Launcher goes at the root (as agentmux.exe — what the user would double-click)
LAUNCHER="$REPO_ROOT/target/release/agentmux-launcher.exe"
if [ ! -f "$LAUNCHER" ]; then
    echo "ERROR: $LAUNCHER not found — run 'task build:host' first" >&2
    exit 1
fi
cp "$LAUNCHER" "$TEST_DIR/agentmux.exe"

# CEF host goes in runtime/ — launcher looks for agentmux-{version}.exe
CEF_HOST="$REPO_ROOT/target/release/agentmux-cef.exe"
if [ ! -f "$CEF_HOST" ]; then
    echo "ERROR: $CEF_HOST not found — run 'task build:host' first" >&2
    exit 1
fi
cp "$CEF_HOST" "$RUNTIME_DIR/agentmux-${VERSION}.exe"

# Symlink or copy all runtime DLLs from dist/cef/ into runtime/
DIST_CEF="$REPO_ROOT/dist/cef"
if [ ! -f "$DIST_CEF/libcef.dll" ]; then
    echo "ERROR: dist/cef/libcef.dll not found — run 'task bundle' first" >&2
    exit 1
fi
cp -f "$DIST_CEF"/*.dll "$RUNTIME_DIR/" 2>/dev/null || true
cp -f "$DIST_CEF"/*.dat "$RUNTIME_DIR/" 2>/dev/null || true
cp -f "$DIST_CEF"/*.bin "$RUNTIME_DIR/" 2>/dev/null || true
cp -f "$DIST_CEF"/*.pak "$RUNTIME_DIR/" 2>/dev/null || true
[ -f "$DIST_CEF/locales/en-US.pak" ] && cp -f "$DIST_CEF/locales/en-US.pak" "$RUNTIME_DIR/locales/"

# Copy srv binary — prefer freshly built matching-version binary,
# fall back to whatever is in dist/bin/ for convenience.
BUILT_SRV="$REPO_ROOT/target/release/agentmux-srv.exe"
if [ -f "$BUILT_SRV" ]; then
    cp "$BUILT_SRV" "$RUNTIME_DIR/agentmux-srv-${VERSION}-windows.x64.exe"
else
    SRV=$(ls "$REPO_ROOT/dist/bin/"agentmux-srv-*-windows.x64.exe 2>/dev/null | head -1)
    [ -n "$SRV" ] && cp "$SRV" "$RUNTIME_DIR/"
fi

echo "==> Layout:"
echo "    $TEST_DIR/agentmux.exe            (launcher — shows splash)"
echo "    $RUNTIME_DIR/agentmux-${VERSION}.exe  (CEF host)"
echo "    $RUNTIME_DIR/libcef.dll + friends"
echo ""
echo "==> Launching via launcher (splash should appear immediately)..."
echo "    URL: $URL"
echo ""

# DEV flag so it uses ~/.agentmux-dev data dir (isolated from prod)
cd "$TEST_DIR"
AGENTMUX_DEV=1 ./agentmux.exe "--url=$URL"
