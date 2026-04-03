#!/usr/bin/env bash
# Package AgentMux CEF as a portable directory + ZIP (Windows x64).
# Usage: bash scripts/package-cef-portable.sh [output-dir]
#
# Default output: ~/Desktop/agentmux-cef-{version}-x64-portable/

set -euo pipefail

VERSION=$(node -p "require('./package.json').version")
OUTDIR="${1:-$HOME/Desktop}"
PORTABLE="$OUTDIR/agentmux-cef-$VERSION-x64-portable"
ZIPPATH="$OUTDIR/agentmux-cef-$VERSION-x64-portable.zip"

echo "Packaging AgentMux CEF v$VERSION Portable..."

# Verify required files — hard fail, no warnings
CEF_SRC="dist/cef/agentmux-cef-${VERSION}-windows.x64.exe"
LAUNCHER_SRC="dist/cef/agentmux-launcher-${VERSION}-windows.x64.exe"
SRV_SRC="dist/bin/agentmux-srv-${VERSION}-windows.x64.exe"
WSH_SRC="dist/bin/agentmux-wsh-${VERSION}-windows.x64.exe"

for f in "$CEF_SRC" "$LAUNCHER_SRC" "$SRV_SRC" "$WSH_SRC" dist/cef/libcef.dll dist/frontend/index.html; do
    if [ ! -f "$f" ]; then
        echo "ERROR: $f not found — build first" >&2
        echo "  Run: task cef:build && task build:backend && task build:frontend:cef" >&2
        exit 1
    fi
done

# Clean previous
rm -rf "$PORTABLE" "$ZIPPATH"

# Create structure
mkdir -p "$PORTABLE/runtime/locales" "$PORTABLE/runtime/frontend"

# Launcher at root (versioned name — visible in Task Manager as agentmux-{VERSION}.exe)
cp "$LAUNCHER_SRC" "$PORTABLE/agentmux-${VERSION}.exe"

# README
cat > "$PORTABLE/README.txt" <<READMEEOF
AgentMux v$VERSION - Portable Edition

Quick Start:
  1. Extract this folder (or ZIP) anywhere
  2. Run agentmux-${VERSION}.exe

Requirements:
  - Windows 10/11 x64
  - No installation needed
  - No admin rights required
READMEEOF

# Runtime binaries (versioned names, no platform suffix — discoverable by glob)
cp "$CEF_SRC" "$PORTABLE/runtime/agentmux-cef-${VERSION}.exe"
cp "$SRV_SRC" "$PORTABLE/runtime/agentmux-srv-${VERSION}.exe"
cp "$WSH_SRC" "$PORTABLE/runtime/agentmux-wsh-${VERSION}.exe"

# Frontend
cp -r dist/frontend/* "$PORTABLE/runtime/frontend/"

# CEF core
cp dist/cef/libcef.dll "$PORTABLE/runtime/"
cp dist/cef/chrome_elf.dll "$PORTABLE/runtime/" 2>/dev/null || true
cp dist/cef/icudtl.dat "$PORTABLE/runtime/" 2>/dev/null || true
cp dist/cef/v8_context_snapshot.bin "$PORTABLE/runtime/" 2>/dev/null || true

# GPU support
cp dist/cef/libEGL.dll dist/cef/libGLESv2.dll dist/cef/d3dcompiler_47.dll "$PORTABLE/runtime/" 2>/dev/null || true

# Resource paks
cp dist/cef/chrome_100_percent.pak dist/cef/chrome_200_percent.pak dist/cef/resources.pak "$PORTABLE/runtime/" 2>/dev/null || true

# Locale (en-US only)
cp dist/cef/locales/en-US.pak "$PORTABLE/runtime/locales/" 2>/dev/null || true

# Verify all versioned binaries are present in the output
for f in \
    "$PORTABLE/agentmux-${VERSION}.exe" \
    "$PORTABLE/runtime/agentmux-cef-${VERSION}.exe" \
    "$PORTABLE/runtime/agentmux-srv-${VERSION}.exe" \
    "$PORTABLE/runtime/agentmux-wsh-${VERSION}.exe"; do
    if [ ! -f "$f" ]; then
        echo "ERROR: Expected output binary missing: $f" >&2
        exit 1
    fi
done

# Size
DIR_SIZE=$(du -sh "$PORTABLE" | cut -f1)

# ZIP
cd "$OUTDIR"
ZIP_NAME="agentmux-cef-$VERSION-x64-portable.zip"
powershell -Command "Compress-Archive -Path '$(basename "$PORTABLE")/*' -DestinationPath '$ZIP_NAME' -Force" 2>/dev/null || true
ZIP_SIZE=$(du -sh "$ZIP_NAME" 2>/dev/null | cut -f1 || echo "N/A")

echo ""
echo "[SUCCESS] CEF Portable v$VERSION"
echo "  Directory: $PORTABLE ($DIR_SIZE)"
echo "  ZIP: $ZIPPATH ($ZIP_SIZE)"
echo ""
echo "  Binaries:"
echo "    agentmux-${VERSION}.exe          (launcher)"
echo "    runtime/agentmux-cef-${VERSION}.exe   (CEF host)"
echo "    runtime/agentmux-srv-${VERSION}.exe   (backend sidecar)"
echo "    runtime/agentmux-wsh-${VERSION}.exe   (shell integration)"
