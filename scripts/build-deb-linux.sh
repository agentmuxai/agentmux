#!/usr/bin/env bash
# Build AgentMux as a .deb package on Linux x86_64, via fpm.
#
# Part of Phase 1, docs/specs/SPEC_LINUX_DISTRO_TARGETS_AND_DOWNLOADS_PAGE_2026_09_15.md.
#
# Usage:
#   bash scripts/build-deb-linux.sh [output-dir]
#
#   output-dir defaults to ~/Desktop. The package is named
#   AgentMux_{version}_amd64.deb in that directory.
#
# Prerequisites (one-time):
#   - fpm on PATH (`gem install fpm`).
#   - Same build artifacts stage-linux-runtime.sh requires (dist/, target/release/).
#
# Dependency policy (spec OQ4, unresolved as an open question at the time of
# writing — this script's default): bundle everything statically, the same
# way the AppImage does, rather than declaring `Depends:` against system GTK/
# Wayland/X11 libraries. This keeps `apt install ./agentmux.deb` from failing
# on a minimal system that's missing one of those, at the cost of a larger
# package than a "thin" .deb that relies on the system's own copies. If OQ4
# is later resolved the other way, add `--deb-no-default-config-files` /
# `--depends <pkg>` flags to the fpm invocation below — no staging changes
# needed, since the staged tree already bundles every runtime lib either way.
#
# Install layout (distinct from the AppImage's self-contained AppDir/AppRun
# scheme — this is what actually installing a .deb puts on the filesystem):
#   /opt/agentmux/usr/bin/...          → the staged runtime (identical
#                                         contents to the AppImage's usr/bin/,
#                                         just at a different mount point)
#   /usr/bin/agentmux                  → thin wrapper exec'ing the launcher
#   /usr/share/applications/agentmux.desktop
#   /usr/share/icons/hicolor/*/apps/agentmux.png

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(node -p "require('./package.json').version")"
OUTDIR="${1:-$HOME/Desktop}"
PKGROOT="$REPO_ROOT/build/AgentMux.deb-root"
# Same local-build-label convention as build-appimage-linux.sh: a labeled
# filename for local builds (so repeat local builds don't collide on disk
# and stay traceable to a branch/commit/timestamp), the bare version for
# release builds (RELEASE_CHANNEL=stable, no AGENTMUX_BUILD_LABEL).
if [ -n "${AGENTMUX_BUILD_LABEL:-}" ]; then
    OUTPUT="$OUTDIR/AgentMux_${AGENTMUX_BUILD_LABEL}_amd64.deb"
else
    OUTPUT="$OUTDIR/AgentMux_${VERSION}_amd64.deb"
fi

if ! command -v fpm >/dev/null 2>&1; then
    echo "ERROR: fpm not found on PATH — install with: gem install fpm" >&2
    exit 1
fi

echo "Building AgentMux v$VERSION .deb → $OUTPUT"

# --- 1. Stage the shared runtime under /opt/agentmux (third-party-app
#        convention — avoids /usr/bin filename collisions with anything
#        else named agentmux-* on the target system, and matches how most
#        non-distro-packaged desktop apps install on Debian/Ubuntu). ---
rm -rf "$PKGROOT"
mkdir -p "$PKGROOT/opt/agentmux"
bash "$REPO_ROOT/scripts/stage-linux-runtime.sh" "$PKGROOT/opt/agentmux"
# stage-linux-runtime.sh populates $PKGROOT/opt/agentmux/usr/bin/... — flatten
# one level since /opt/agentmux doesn't need a nested usr/ of its own.
mv "$PKGROOT/opt/agentmux/usr"/* "$PKGROOT/opt/agentmux/"
rmdir "$PKGROOT/opt/agentmux/usr"

# --- 2. /usr/bin wrapper — the actual `agentmux` command on PATH after
#        install. Thin exec into the launcher; no AppImage-style env setup
#        needed since this is a real filesystem install, not a mounted
#        squashfs. ---
mkdir -p "$PKGROOT/usr/bin"
cat > "$PKGROOT/usr/bin/agentmux" <<'WRAP'
#!/usr/bin/env bash
exec /opt/agentmux/bin/agentmux-launcher "$@"
WRAP
chmod +x "$PKGROOT/usr/bin/agentmux"

# --- 3. Desktop entry + icons — real installed-system desktop integration,
#        distinct from the AppImage's self-registering install-linux-desktop.sh
#        (which exists specifically because an AppImage has no privileged
#        install step to place these itself). A .deb's postinst has no such
#        constraint — these just go straight into their standard locations. ---
mkdir -p "$PKGROOT/usr/share/applications"
cp assets/linux/agentmux.desktop "$PKGROOT/usr/share/applications/agentmux.desktop"
sed -i 's|^Exec=.*|Exec=/usr/bin/agentmux %F|' "$PKGROOT/usr/share/applications/agentmux.desktop"

for size in 16 32 48 64 128 256 512; do
    src="assets/linux/icons/hicolor/${size}x${size}/apps/agentmux.png"
    dst="$PKGROOT/usr/share/icons/hicolor/${size}x${size}/apps/agentmux.png"
    if [ -f "$src" ]; then
        mkdir -p "$(dirname "$dst")"
        cp "$src" "$dst"
    fi
done

# --- 4. Package with fpm — turns $PKGROOT (already laid out exactly as it
#        should land on the target filesystem) into a .deb. No Depends: per
#        this script's header comment; revisit per spec OQ4 if needed. ---
mkdir -p "$OUTDIR"
rm -f "$OUTPUT"
fpm -s dir -t deb \
    --name agentmux \
    --version "$VERSION" \
    --architecture amd64 \
    --maintainer "AgentMux <noreply@agentmux.ai>" \
    --description "AgentMux — Agent Operating Environment" \
    --url "https://agentmux.ai" \
    --license Apache-2.0 \
    --category utils \
    --package "$OUTPUT" \
    -C "$PKGROOT" .

echo ""
echo "✓ Built .deb: $OUTPUT"
ls -lh "$OUTPUT"
