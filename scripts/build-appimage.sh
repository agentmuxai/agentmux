#!/bin/bash
# Build a portable Linux AppImage for AgentMux.
#
# Workaround: Tauri's built-in AppImage bundler uses linuxdeploy which crashes
# on some systems. This script lets Tauri prepare the AppDir, then finishes
# packaging with appimagetool directly.
#
# Usage:
#   bash scripts/build-appimage.sh
#
# Requirements:
#   appimagetool in PATH (https://github.com/AppImage/appimagetool)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# Resolve version dynamically
VERSION=$(node version.cjs)
APPDIR="src-tauri/target/release/bundle/appimage/AgentMux.AppDir"
OUTPUT="AgentMux_${VERSION}_amd64.AppImage"

echo "========================================="
echo "AgentMux Linux AppImage Build v${VERSION}"
echo "========================================="

if ! command -v appimagetool &>/dev/null; then
    echo "❌ appimagetool not found in PATH"
    echo "   Download: https://github.com/AppImage/appimagetool/releases"
    exit 1
fi

# Step 1: Build Rust backend (agentmux-srv)
echo ""
echo "Step 1: Building Rust backend..."
task build:backend:rust
echo "✅ Rust backend built"

# Step 2: Build Go backend + wsh, copy sidecars
echo ""
echo "Step 2: Building Go backend + copying sidecars..."
task build:backend
task tauri:copy-sidecars
echo "✅ Go backend + sidecars ready"

# Step 3: Let Tauri prepare the AppDir (fails at linuxdeploy — that's expected)
echo ""
echo "Step 3: Preparing AppDir via Tauri build..."
npx tauri build --bundles appimage 2>&1 | grep -v "failed to run linuxdeploy" || true

if [ ! -d "$APPDIR" ]; then
    echo "❌ AppDir not created — Tauri build failed before linuxdeploy stage"
    exit 1
fi
echo "✅ AppDir prepared"

# Step 4: Fix icon name (desktop file references lowercase 'agentmux')
echo ""
echo "Step 4: Ensuring icon..."
[ -f "$APPDIR/agentmux.png" ] || cp "$APPDIR/AgentMux.png" "$APPDIR/agentmux.png"
echo "✅ Icon ready"

# Step 5: Create AppImage with appimagetool
echo ""
echo "Step 5: Creating AppImage..."
cd "$(dirname "$APPDIR")"
ARCH=x86_64 appimagetool "AgentMux.AppDir" "$OUTPUT"
echo "✅ AppImage created: $OUTPUT"

# Step 6: Copy to Desktop
echo ""
echo "Step 6: Copying to Desktop..."
if [ -d "$HOME/Desktop" ]; then
    cp "$OUTPUT" "$HOME/Desktop/$OUTPUT"
    echo "✅ $OUTPUT → Desktop"
else
    echo "ℹ No Desktop directory found, AppImage is at: $(pwd)/$OUTPUT"
fi

echo ""
echo "========================================="
echo "✅ BUILD COMPLETE: AgentMux ${VERSION}"
echo "========================================="
