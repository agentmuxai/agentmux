#!/usr/bin/env bash
# Verify packaged build contains all required binaries and resources
# This script should be run AFTER electron-builder completes packaging

set -eo pipefail

# Colors
GREEN='\033[0;32m'
CYAN='\033[0;36m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

success() { echo -e "${GREEN}✓ $1${NC}"; }
info() { echo -e "${CYAN}→ $1${NC}"; }
error() { echo -e "${RED}✗ $1${NC}" >&2; }
warn() { echo -e "${YELLOW}⚠ $1${NC}"; }

ISSUES=0
VERSION=$(node -p "require('./package.json').version")

info "Verifying package for version $VERSION"
echo ""

# Determine package location based on platform
if [[ -d "make/win-unpacked" ]]; then
    PACKAGE_DIR="make/win-unpacked"
    EXE_NAME="AgentMux.exe"
elif [[ -d "make/darwin" ]]; then
    PACKAGE_DIR="make/darwin/AgentMux.app/Contents"
    EXE_NAME="MacOS/Wave"
elif [[ -d "make/linux-unpacked" ]]; then
    PACKAGE_DIR="make/linux-unpacked"
    EXE_NAME="agentmux"
else
    error "No unpacked package found in make/"
    exit 1
fi

info "Package location: $PACKAGE_DIR"
echo ""

# Check main executable
if [[ -f "$PACKAGE_DIR/$EXE_NAME" ]]; then
    success "Main executable: $EXE_NAME"
else
    error "Main executable missing: $EXE_NAME"
    ((ISSUES++))
fi

# Check agentmuxsrv binary
WAVESRV_PATH="$PACKAGE_DIR/resources/app.asar.unpacked/dist/bin"
if [[ -d "$WAVESRV_PATH" ]]; then
    # Check for agentmuxsrv
    if ls "$WAVESRV_PATH"/agentmuxsrv* 2>/dev/null | grep -q .; then
        WAVESRV=$(ls "$WAVESRV_PATH"/agentmuxsrv* 2>/dev/null | head -1)
        success "agentmuxsrv: $(basename "$WAVESRV")"
    else
        error "agentmuxsrv binary missing in: $WAVESRV_PATH"
        ((ISSUES++))
    fi

    # Check for wsh with correct version
    if ls "$WAVESRV_PATH"/wsh-${VERSION}-* 2>/dev/null | grep -q .; then
        WSH_COUNT=$(ls "$WAVESRV_PATH"/wsh-${VERSION}-* 2>/dev/null | wc -l)
        success "wsh binaries: $WSH_COUNT files for version $VERSION"
        ls "$WAVESRV_PATH"/wsh-${VERSION}-* 2>/dev/null | while read file; do
            echo "    - $(basename "$file")"
        done
    else
        error "wsh binaries missing or wrong version in: $WAVESRV_PATH"
        error "Expected: wsh-${VERSION}-*"
        echo "  Found:"
        ls "$WAVESRV_PATH"/wsh-* 2>/dev/null || echo "    (none)"
        ((ISSUES++))
    fi
else
    error "Binary directory missing: $WAVESRV_PATH"
    ((ISSUES++))
fi

# Check ASAR contents
if [[ -f "$PACKAGE_DIR/resources/app.asar" ]]; then
    info "Checking ASAR contents..."

    # Extract ASAR to temp location
    TEMP_EXTRACT="/tmp/agentmux-asar-verify-$$"
    mkdir -p "$TEMP_EXTRACT"
    npx asar extract "$PACKAGE_DIR/resources/app.asar" "$TEMP_EXTRACT" 2>/dev/null

    # Check package.json
    if [[ -f "$TEMP_EXTRACT/package.json" ]]; then
        PKG_VERSION=$(node -p "require('$TEMP_EXTRACT/package.json').version" 2>/dev/null)
        if [[ "$PKG_VERSION" == "$VERSION" ]]; then
            success "ASAR package.json: version $PKG_VERSION"
        else
            error "ASAR package.json version mismatch: $PKG_VERSION (expected: $VERSION)"
            ((ISSUES++))
        fi
    else
        error "ASAR missing package.json"
        ((ISSUES++))
    fi

    # Check main entry point
    if [[ -f "$TEMP_EXTRACT/dist/main/index.js" ]]; then
        success "ASAR main entry: dist/main/index.js"
    else
        error "ASAR missing main entry: dist/main/index.js"
        ((ISSUES++))
    fi

    # Check frontend
    if [[ -f "$TEMP_EXTRACT/dist/frontend/index.html" ]]; then
        success "ASAR frontend: dist/frontend/index.html"
    else
        error "ASAR missing frontend: dist/frontend/index.html"
        ((ISSUES++))
    fi

    # Clean up
    rm -rf "$TEMP_EXTRACT"
else
    error "app.asar missing: $PACKAGE_DIR/resources/app.asar"
    ((ISSUES++))
fi

echo ""

# Check for critical shell integration files
info "Checking shell integration files..."
SHELL_DIR="$PACKAGE_DIR/resources/app.asar.unpacked/dist/bin"
if [[ -d "$SHELL_DIR" ]]; then
    # These files are critical for shell integration to work
    SHELL_FILES=()

    # On Windows, check for PowerShell integration
    if [[ "$EXE_NAME" == "AgentMux.exe" ]]; then
        # wsh must be present for PowerShell integration
        if ls "$SHELL_DIR"/wsh-${VERSION}-windows*.exe 2>/dev/null | grep -q .; then
            success "Shell integration: wsh Windows binaries present"
        else
            error "Shell integration: wsh Windows binaries missing"
            error "PowerShell integration will fail without wsh in PATH"
            ((ISSUES++))
        fi
    fi
else
    warn "Shell integration directory not found (may be expected)"
fi

echo ""

# Summary
if [[ $ISSUES -eq 0 ]]; then
    success "Package verification passed! ✓"
    success "All required binaries and resources present for version $VERSION"
    exit 0
else
    error "Package verification failed with $ISSUES issues"
    error "DO NOT distribute this package - it is missing critical components"
    exit 1
fi
