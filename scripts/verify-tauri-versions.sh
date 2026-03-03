#!/bin/bash
set -e

# Change to project root
cd "$(dirname "$0")/.." || exit 1

echo "Checking Tauri version alignment..."
echo

# Extract versions using node (jq may not be available)
NPM_CLI=$(node -e "try{const p=require('./node_modules/@tauri-apps/cli/package.json');console.log(p.version)}catch{console.log('not installed')}" 2>/dev/null || echo "not installed")
NPM_API=$(node -e "try{const p=require('./node_modules/@tauri-apps/api/package.json');console.log(p.version)}catch{console.log('not installed')}" 2>/dev/null || echo "not installed")

# Cargo.lock is at workspace root, not src-tauri/
CARGO_LOCK="Cargo.lock"
if [ ! -f "$CARGO_LOCK" ]; then
    CARGO_LOCK="src-tauri/Cargo.lock"
fi
CARGO_TAURI=$(awk '/^\[\[package\]\]/{p=0} /^name = "tauri"$/{p=1} p && /^version =/{print $3; exit}' "$CARGO_LOCK" 2>/dev/null | tr -d '"' || echo "not found")
if [ -z "$CARGO_TAURI" ]; then
    CARGO_TAURI="not found"
fi

# Extract major.minor
if [ "$NPM_CLI" != "not installed" ]; then
    NPM_CLI_MM=$(echo $NPM_CLI | cut -d. -f1,2)
else
    NPM_CLI_MM="N/A"
fi

if [ "$NPM_API" != "not installed" ]; then
    NPM_API_MM=$(echo $NPM_API | cut -d. -f1,2)
else
    NPM_API_MM="N/A"
fi

if [ "$CARGO_TAURI" != "not found" ]; then
    CARGO_MM=$(echo $CARGO_TAURI | cut -d. -f1,2)
else
    CARGO_MM="N/A"
fi

echo "Installed Versions:"
echo "  @tauri-apps/cli: $NPM_CLI (major.minor: $NPM_CLI_MM)"
echo "  @tauri-apps/api: $NPM_API (major.minor: $NPM_API_MM)"
echo "  tauri crate:     $CARGO_TAURI (major.minor: $CARGO_MM)"
echo

# Verify all match
if [ "$NPM_CLI_MM" = "N/A" ] || [ "$NPM_API_MM" = "N/A" ] || [ "$CARGO_MM" = "N/A" ]; then
    echo "ERROR: Some Tauri packages are not installed!"
    echo "   Run 'npm install' and 'cargo build' first."
    exit 1
fi

if [ "$NPM_CLI_MM" != "$NPM_API_MM" ] || [ "$NPM_CLI_MM" != "$CARGO_MM" ]; then
    echo "ERROR: Tauri version mismatch!"
    echo
    echo "   All Tauri packages must be on the same major.minor version."
    echo "   Expected: All on $NPM_CLI_MM.x"
    echo
    echo "   To fix:"
    echo "   1. Update package.json to pin versions (remove ^)"
    echo "   2. Update Cargo.toml to pin version (use =MAJOR.MINOR)"
    echo "   3. Run: npm install && cd src-tauri && cargo update tauri"
    echo
    echo "   Or use: ./scripts/update-tauri.sh <version>"
    exit 1
fi

echo "All Tauri core packages aligned on $NPM_CLI_MM.x"
echo

# Check plugin versions
echo "Checking Tauri plugin alignment..."
echo

PLUGIN_ERRORS=0

PLUGINS_WITH_NPM=(
    "shell"
    "opener"
    "fs"
    "notification"
)

for plugin in "${PLUGINS_WITH_NPM[@]}"; do
    npm_ver=$(node -e "try{const p=require('./node_modules/@tauri-apps/plugin-$plugin/package.json');console.log(p.version)}catch{console.log('not installed')}" 2>/dev/null || echo "not installed")
    cargo_ver=$(awk "/^\[\[package\]\]/{p=0} /^name = \"tauri-plugin-$plugin\"$/{p=1} p && /^version =/{print \$3; exit}" "$CARGO_LOCK" 2>/dev/null | tr -d '"' || echo "not found")

    if [ -z "$cargo_ver" ]; then
        cargo_ver="not found"
    fi

    if [ "$npm_ver" = "not installed" ]; then
        echo "  WARN: @tauri-apps/plugin-$plugin: not installed in npm"
        continue
    fi

    if [ "$cargo_ver" = "not found" ]; then
        echo "  WARN: tauri-plugin-$plugin: not found in Cargo.lock"
        continue
    fi

    npm_mm=$(echo $npm_ver | cut -d. -f1,2)
    cargo_mm=$(echo $cargo_ver | cut -d. -f1,2)

    if [ "$npm_mm" != "$cargo_mm" ]; then
        echo "  FAIL: plugin-$plugin: npm $npm_ver (${npm_mm}.x) != cargo $cargo_ver (${cargo_mm}.x)"
        PLUGIN_ERRORS=$((PLUGIN_ERRORS + 1))
    else
        echo "  OK: plugin-$plugin: npm $npm_ver, cargo $cargo_ver (${npm_mm}.x)"
    fi
done

echo

if [ $PLUGIN_ERRORS -gt 0 ]; then
    echo "ERROR: $PLUGIN_ERRORS plugin version mismatch(es) found!"
    echo
    echo "   To fix:"
    echo "   1. Update package.json plugin versions to match Cargo.lock"
    echo "   2. Update Cargo.toml plugin versions (use =MAJOR.MINOR)"
    echo "   3. Run: npm install && cd src-tauri && cargo update"
    echo
    echo "   Or use: ./scripts/update-tauri.sh <version> --plugins"
    exit 1
fi

echo "All Tauri packages and plugins aligned!"
echo "   Build should succeed!"
