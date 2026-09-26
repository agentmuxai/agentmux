#!/usr/bin/env bash
# Build AgentMux as a .rpm package on Linux x86_64, via fpm.
#
# Part of Phase 2, docs/specs/SPEC_LINUX_DISTRO_TARGETS_AND_DOWNLOADS_PAGE_2026_09_15.md.
# Mirrors build-deb-linux.sh closely — same staged tree, same /opt/agentmux
# install layout, same fpm invocation shape, just `-t rpm` instead of `-t deb`
# and `--iteration`/`--rpm-*` flags where the two formats' metadata differs.
#
# Usage:
#   bash scripts/build-rpm-linux.sh [output-dir]
#
#   output-dir defaults to ~/Desktop. The package is named
#   AgentMux-{version}-1.x86_64.rpm in that directory (rpm's own convention:
#   `-` separators, no distro/codename suffix, an explicit release/iteration
#   number — "1" here, since this is the first packaging of each version).
#
# Prerequisites (one-time):
#   - fpm on PATH (`gem install fpm`).
#   - rpmbuild on PATH (`apt-get install rpm` on Debian/Ubuntu — fpm's `-t rpm`
#     backend shells out to it; unlike `-t deb`, this is NOT pure Ruby).
#   - Same build artifacts stage-linux-runtime.sh requires (dist/, target/release/).
#
# Dependency policy: same as build-deb-linux.sh (spec OQ4, still open) —
# bundle everything statically, no `Requires:` declared. Fedora/openSUSE
# package names for the same runtime libs differ from Debian's, so if OQ4
# is later resolved toward declaring real dependencies, this script and
# build-deb-linux.sh need that pass done together, not one at a time — a
# `.deb` with `Depends:` and a `.rpm` still bundling everything (or vice
# versa) would be an inconsistent policy across the two formats for no
# reason a user could see.
#
# Install layout (identical to build-deb-linux.sh's — see that script's
# header for the full rationale):
#   /opt/agentmux/usr/bin/...          → the staged runtime
#   /usr/bin/agentmux                  → thin wrapper exec'ing the launcher
#   /usr/share/applications/agentmux.desktop
#   /usr/share/icons/hicolor/*/apps/agentmux.png

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(node -p "require('./package.json').version")"
OUTDIR="${1:-$HOME/Desktop}"
PKGROOT="$REPO_ROOT/build/AgentMux.rpm-root"
# rpm filenames conventionally use hyphens and an explicit release/iteration
# number, distinct from the deb/AppImage/tarball naming — this is rpm's own
# packaging convention, not an AgentMux choice.
if [ -n "${AGENTMUX_BUILD_LABEL:-}" ]; then
    OUTPUT="$OUTDIR/AgentMux-${AGENTMUX_BUILD_LABEL}-1.x86_64.rpm"
else
    OUTPUT="$OUTDIR/AgentMux-${VERSION}-1.x86_64.rpm"
fi

if ! command -v fpm >/dev/null 2>&1; then
    echo "ERROR: fpm not found on PATH — install with: gem install fpm" >&2
    exit 1
fi
if ! command -v rpmbuild >/dev/null 2>&1; then
    echo "ERROR: rpmbuild not found on PATH — install with: apt-get install rpm (Debian/Ubuntu) or dnf install rpm-build (Fedora)" >&2
    exit 1
fi

echo "Building AgentMux v$VERSION .rpm → $OUTPUT"

# --- 1. Stage the shared runtime under /opt/agentmux — identical to
#        build-deb-linux.sh's step 1; see that script for the rationale. ---
rm -rf "$PKGROOT"
mkdir -p "$PKGROOT/opt/agentmux"
bash "$REPO_ROOT/scripts/stage-linux-runtime.sh" "$PKGROOT/opt/agentmux"
mv "$PKGROOT/opt/agentmux/usr"/* "$PKGROOT/opt/agentmux/"
rmdir "$PKGROOT/opt/agentmux/usr"

# --- 1b. AppArmor userns-fix helper — identical to build-deb-linux.sh's
#         step 1b; see that script for the full rationale. ---
cp scripts/install-userns-apparmor-fix.sh "$PKGROOT/opt/agentmux/bin/install-userns-apparmor-fix.sh"
chmod +x "$PKGROOT/opt/agentmux/bin/install-userns-apparmor-fix.sh"

# --- 2. /usr/bin wrapper — identical to build-deb-linux.sh's step 2,
#        including the LD_LIBRARY_PATH export (Codex P1 on PR #3236 caught
#        this missing from the .deb's wrapper originally; same fix applies
#        here since both formats share the /opt/agentmux/bin layout). ---
mkdir -p "$PKGROOT/usr/bin"
cat > "$PKGROOT/usr/bin/agentmux" <<'WRAP'
#!/usr/bin/env bash
export LD_LIBRARY_PATH="/opt/agentmux/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
# A start-at-login entry must run this wrapper, not the raw launcher (which
# cannot find libcef.so without the line above).
export AGENTMUX_STABLE_EXE=/usr/bin/agentmux
exec /opt/agentmux/bin/agentmux-launcher "$@"
WRAP
chmod +x "$PKGROOT/usr/bin/agentmux"

# --- 3. Desktop entry + icons — identical to build-deb-linux.sh's step 3. ---
mkdir -p "$PKGROOT/usr/share/applications"
cp assets/linux/agentmux.desktop "$PKGROOT/usr/share/applications/agentmux.desktop"
sed -i 's|^Exec=.*|Exec=/usr/bin/agentmux %F|' "$PKGROOT/usr/share/applications/agentmux.desktop"
# StartupWMClass must match the app_id the bundled binary actually
# advertises (window_settings.rs::linux_app_id() = agentmux-<channel>-
# <version>). `task package:linux:rpm` (package-linux.sh --format=rpm)
# exports AGENTMUX_BUILD_CHANNEL_DEFAULT to a real per-build local-*
# channel before compiling — same as the AppImage path — so this must read
# it rather than assume "stable"; only `task package:release:linux:rpm`
# (RELEASE_CHANNEL=stable) actually bakes "stable".
BUILD_CHANNEL="${AGENTMUX_BUILD_CHANNEL_DEFAULT:-stable}"
sed -i "s|__WMCLASS__|agentmux-${BUILD_CHANNEL}-${VERSION}|" "$PKGROOT/usr/share/applications/agentmux.desktop"

for size in 16 32 48 64 128 256 512; do
    src="assets/linux/icons/hicolor/${size}x${size}/apps/agentmux.png"
    dst="$PKGROOT/usr/share/icons/hicolor/${size}x${size}/apps/agentmux.png"
    if [ -f "$src" ]; then
        mkdir -p "$(dirname "$dst")"
        cp "$src" "$dst"
    fi
done

# --- 4. Package with fpm -t rpm. --iteration sets the release number (the
#        "-1" in the filename); --rpm-os linux keeps fpm from trying to
#        infer a target from the build host. No Requires: — see this
#        script's header comment. ---
mkdir -p "$OUTDIR"
rm -f "$OUTPUT"
fpm -s dir -t rpm \
    --name agentmux \
    --version "$VERSION" \
    --iteration 1 \
    --architecture x86_64 \
    --rpm-os linux \
    --maintainer "AgentMux <noreply@agentmux.ai>" \
    --description "AgentMux — Agent Operating Environment" \
    --url "https://agentmux.ai" \
    --license Apache-2.0 \
    --category Utilities \
    --package "$OUTPUT" \
    -C "$PKGROOT" .

echo ""
echo "✓ Built .rpm: $OUTPUT"
ls -lh "$OUTPUT"
