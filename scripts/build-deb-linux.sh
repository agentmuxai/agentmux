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
# Dependencies: the CEF runtime links against system libraries the package does not carry
# (NSS, GTK's ATK, X11 and XKB, ALSA, libdrm's gbm, cups, Wayland, ...). On a minimal system
# `agentmux --version` failed for want of them, so the package declares them in `Depends:`.
# scripts/linux-package-deps.list is the one list, scripts/linux-package-deps.sh turns it
# into the fpm flags below, and refuses to build when the staged tree needs a library that is
# neither bundled nor listed.
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

# --- 1b. AppArmor userns-fix helper (Codex P1, PR #3236) —
#         linux_sandbox::resolve_helper_script_path() prefers $APPDIR (set
#         by the AppImage's AppRun, never set here) and otherwise falls
#         back to a path relative to agentmux-cef's own binary directory —
#         i.e. exactly /opt/agentmux/bin/ once the flatten above runs, so
#         placing the script there (not the AppImage's AppDir-root
#         convention) is what makes that fallback actually resolve.
#         build_apparmor_profile() now includes a stanza for
#         /opt/agentmux/bin/agentmux-cef (crates/cef/src/linux_sandbox.rs)
#         so "Fix it now" actually grants userns at this install's real path
#         instead of silently succeeding without doing so. ---
cp scripts/install-userns-apparmor-fix.sh "$PKGROOT/opt/agentmux/bin/install-userns-apparmor-fix.sh"
chmod +x "$PKGROOT/opt/agentmux/bin/install-userns-apparmor-fix.sh"

# --- 1c. The in-app upgrade's helper and its polkit policy
#         (SPEC_LINUX_PACKAGE_UPGRADE_AND_REPOSITORY_2026_10_24.md §3.2, §3.3):
#         AgentMux runs `pkexec /opt/agentmux/bin/agentmux-install-update`, the desktop
#         asks for the password, and only this root-owned script (which checks the file
#         itself) installs the update. The policy names that exact path. ---
cp scripts/agentmux-install-update.sh "$PKGROOT/opt/agentmux/bin/agentmux-install-update"
chmod 755 "$PKGROOT/opt/agentmux/bin/agentmux-install-update"
mkdir -p "$PKGROOT/usr/share/polkit-1/actions"
cp assets/linux/ai.agentmux.install-update.policy "$PKGROOT/usr/share/polkit-1/actions/ai.agentmux.install-update.policy"
chmod 644 "$PKGROOT/usr/share/polkit-1/actions/ai.agentmux.install-update.policy"

# --- 2. /usr/bin wrapper — the actual `agentmux` command on PATH after
#        install. No AppImage runtime/extract-once-cache needed since this
#        is a real filesystem install, but LD_LIBRARY_PATH still is:
#        agentmux-cef is built without RPATH (same as the AppImage's
#        agentmux-cef — see scripts/linux-apprun.sh's identical comment),
#        so without this the launcher's exec fails to find libcef.so.
#        Codex P1, PR #3236. ---
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

# --- 3. Desktop entry + icons — real installed-system desktop integration,
#        distinct from the AppImage's self-registering install-linux-desktop.sh
#        (which exists specifically because an AppImage has no privileged
#        install step to place these itself). A .deb's postinst has no such
#        constraint — these just go straight into their standard locations. ---
mkdir -p "$PKGROOT/usr/share/applications"
cp assets/linux/agentmux.desktop "$PKGROOT/usr/share/applications/agentmux.desktop"
sed -i 's|^Exec=.*|Exec=/usr/bin/agentmux %F|' "$PKGROOT/usr/share/applications/agentmux.desktop"
# StartupWMClass must match the app_id the bundled binary actually
# advertises (window_settings.rs::linux_app_id() = agentmux-<channel>-
# <version>). `task package:linux:deb` (package-linux.sh --format=deb)
# exports AGENTMUX_BUILD_CHANNEL_DEFAULT to a real per-build local-*
# channel before compiling — same as the AppImage path — so this must read
# it rather than assume "stable"; only `task package:release:linux:deb`
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

# --- 4. Package with fpm — turns $PKGROOT (already laid out exactly as it
#        should land on the target filesystem) into a .deb. Depends: comes from
#        scripts/linux-package-deps.list; the check fails the build when the staged
#        runtime needs a system library the list lacks. ---
bash "$REPO_ROOT/scripts/linux-package-deps.sh" check "$PKGROOT"
DEPENDS_ARGS=()
while IFS= read -r dep; do
    DEPENDS_ARGS+=(--depends "$dep")
done < <(bash "$REPO_ROOT/scripts/linux-package-deps.sh" deb "$PKGROOT")
mkdir -p "$OUTDIR"
rm -f "$OUTPUT"
fpm -s dir -t deb \
    "${DEPENDS_ARGS[@]}" \
    --name agentmux \
    --version "$VERSION" \
    --architecture amd64 \
    --maintainer "AgentMux <noreply@agentmux.ai>" \
    --description "AgentMux — Agent Operating Environment" \
    --url "https://agentmux.ai" \
    --license Apache-2.0 \
    --category utils \
    --deb-user root \
    --deb-group root \
    --deb-compression xz \
    --deb-compression-level 9 \
    --package "$OUTPUT" \
    -C "$PKGROOT" .

echo ""
echo "✓ Built .deb: $OUTPUT"
ls -lh "$OUTPUT"
