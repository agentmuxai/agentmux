#!/bin/bash
set -e

UPDATE_PLUGINS=false

# Parse arguments
if [ -z "$1" ]; then
    echo "Usage: ./scripts/update-tauri.sh <version> [--plugins]"
    echo "Example: ./scripts/update-tauri.sh 2.11.0"
    echo "         ./scripts/update-tauri.sh 2.11.0 --plugins"
    echo
    echo "Options:"
    echo "  --plugins    Also update all Tauri plugin versions"
    exit 1
fi

VERSION=$1
MAJOR_MINOR=$(echo $VERSION | cut -d. -f1,2)

if [ "$2" = "--plugins" ]; then
    UPDATE_PLUGINS=true
fi

echo "🔧 Updating Tauri dependencies to $VERSION..."
echo

# Update package.json (remove ^ and pin exact versions)
echo "📝 Updating core packages in package.json..."
npm install \
    @tauri-apps/cli@$VERSION \
    @tauri-apps/api@$VERSION \
    --save-exact

# Update Cargo.toml
echo "📝 Updating Cargo.toml..."
sed -i.bak "s/tauri = { version = \"=[0-9.]*\"/tauri = { version = \"=$MAJOR_MINOR\"/" src-tauri/Cargo.toml
rm -f src-tauri/Cargo.toml.bak

# Update Cargo.lock
echo "📝 Updating Cargo.lock..."
(cd src-tauri && cargo update tauri)

if [ "$UPDATE_PLUGINS" = true ]; then
    echo
    echo "🔌 Updating Tauri plugins..."
    echo

    # Get list of plugins from Cargo.lock and update them
    PLUGINS_WITH_NPM=(
        "shell"
        "opener"
        "fs"
        "notification"
    )

    for plugin in "${PLUGINS_WITH_NPM[@]}"; do
        # Get version from Cargo.lock
        cargo_ver=$(awk "/^\[\[package\]\]/{p=0} /^name = \"tauri-plugin-$plugin\"$/{p=1} p && /^version =/{print \$3; exit}" src-tauri/Cargo.lock | tr -d '"' 2>/dev/null)

        if [ -n "$cargo_ver" ] && [ "$cargo_ver" != "" ]; then
            echo "  📦 Updating @tauri-apps/plugin-$plugin to $cargo_ver..."
            npm install --save-exact @tauri-apps/plugin-$plugin@$cargo_ver 2>/dev/null || echo "    ⚠️  Package not found or not needed"

            # Update Cargo.toml to pin major.minor
            # Handles both simple format: tauri-plugin-foo = "2"
            # And complex format: tauri-plugin-foo = { version = "2", features = [...] }
            cargo_mm=$(echo $cargo_ver | cut -d. -f1,2)
            sed -i.bak \
                -e "s/tauri-plugin-$plugin = \"[^\"]*\"/tauri-plugin-$plugin = \"=$cargo_mm\"/" \
                -e "s/tauri-plugin-$plugin = { version = \"[^\"]*\"/tauri-plugin-$plugin = { version = \"=$cargo_mm\"/" \
                src-tauri/Cargo.toml
            rm -f src-tauri/Cargo.toml.bak
        fi
    done

    # Update all plugin versions in Cargo.lock
    echo "  📝 Updating Cargo.lock for plugins..."
    (cd src-tauri && cargo update)
fi

echo
echo "✅ Updated Tauri dependencies to $VERSION"
if [ "$UPDATE_PLUGINS" = true ]; then
    echo "✅ Updated plugin versions"
fi
echo

# Verify
echo "🔍 Verifying version alignment..."
./scripts/verify-tauri-versions.sh
