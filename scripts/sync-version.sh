#!/usr/bin/env bash
# Sync version from package.json (SSOT) to all derived files
# This ensures all version references stay consistent

set -euo pipefail

# Colors
GREEN='\033[0;32m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

success() { echo -e "${GREEN}✓ $1${NC}"; }
info() { echo -e "${CYAN}→ $1${NC}"; }
error() { echo -e "${RED}✗ $1${NC}" >&2; }

# Change to repo root
cd "$(dirname "$0")/.."

# 1. Extract version from package.json (Single Source of Truth)
VERSION=$(node -p "require('./package.json').version")
info "Syncing version from package.json: $VERSION"

# 2. Update src-tauri/tauri.conf.json
if [[ -f "src-tauri/tauri.conf.json" ]]; then
    if command -v jq &> /dev/null; then
        # Use jq if available (preserves formatting better)
        jq --arg ver "$VERSION" '.version = $ver' src-tauri/tauri.conf.json > src-tauri/tauri.conf.json.tmp
        mv src-tauri/tauri.conf.json.tmp src-tauri/tauri.conf.json
    else
        # Fallback to sed
        sed -i "s/\"version\": \"[0-9.]*\"/\"version\": \"$VERSION\"/" src-tauri/tauri.conf.json
    fi
    success "Updated src-tauri/tauri.conf.json → $VERSION"
else
    error "src-tauri/tauri.conf.json not found!"
    exit 1
fi

# 3. Update src-tauri/Cargo.toml
if [[ -f "src-tauri/Cargo.toml" ]]; then
    # Update only the first occurrence (package version, not dependencies)
    sed -i "0,/^version = \"[0-9.]*\"/{s/^version = \"[0-9.]*\"/version = \"$VERSION\"/}" src-tauri/Cargo.toml
    success "Updated src-tauri/Cargo.toml → $VERSION"
else
    error "src-tauri/Cargo.toml not found!"
    exit 1
fi

# 4. Update src-tauri/Cargo.lock (proper way: use cargo)
if [[ -f "src-tauri/Cargo.lock" ]]; then
    info "Updating Cargo.lock..."
    (cd src-tauri && cargo update -p agentmux --quiet 2>/dev/null) || {
        error "Failed to update Cargo.lock via cargo"
        exit 1
    }
    success "Updated src-tauri/Cargo.lock → $VERSION"
else
    error "src-tauri/Cargo.lock not found!"
    exit 1
fi

echo ""
success "All version files synced to $VERSION ✓"
echo ""
info "Files updated:"
echo "  • package.json (source)"
echo "  • src-tauri/tauri.conf.json"
echo "  • src-tauri/Cargo.toml"
echo "  • src-tauri/Cargo.lock"
echo ""
info "Next: Run 'bash scripts/verify-version.sh' to validate"
