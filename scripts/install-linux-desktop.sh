#!/usr/bin/env bash
# Install agentmux.desktop + hicolor icon files for the current user so that
# Wayland/X11 window managers (GNOME Shell, KWin, sway) can match the running
# AgentMux window to a desktop entry and display the AgentMux logo in the
# taskbar/dock/launcher.
#
# Idempotent: re-runs are safe and update the .desktop's Exec= line if it
# changed (e.g. switching between dev binary, portable bundle, and AppImage).
#
# The matching contract:
#   - agentmux-cef sets xdg_toplevel.app_id = "agentmux" via the
#     WindowDelegate::linux_window_properties override (agentmux-cef/src/app.rs).
#   - This script installs ~/.local/share/applications/agentmux.desktop —
#     basename matches the app_id.
#   - The .desktop's Icon=agentmux references icons installed under
#     ~/.local/share/icons/hicolor/<size>x<size>/apps/agentmux.png.
#
# Usage:
#   bash scripts/install-linux-desktop.sh <exec-path>
#       <exec-path>  absolute path to the binary or AppImage that the
#                    .desktop's Exec= should point to.

set -euo pipefail

EXEC_PATH="${1:?usage: $0 <absolute-exec-path>}"
case "$EXEC_PATH" in
    /*) ;;
    *) echo "ERROR: <exec-path> must be absolute, got: $EXEC_PATH" >&2; exit 1 ;;
esac

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
APPS_DIR="$HOME/.local/share/applications"
ICONS_ROOT="$HOME/.local/share/icons/hicolor"

# Resolve where the assets/ tree lives. Two valid layouts:
#   1. dev / source tree   — script at <repo>/scripts/, assets at <repo>/assets/linux/
#   2. inside an AppImage  — script + assets/ both at the AppImage root
# Try the AppImage layout first (script_dir/assets/linux), then the dev layout
# (script_dir/../assets/linux). Bail with a clear error if neither resolves.
ASSETS_BASE=""
for candidate in "$SCRIPT_DIR/assets/linux" "$SCRIPT_DIR/../assets/linux"; do
    if [ -f "$candidate/agentmux.desktop" ]; then
        ASSETS_BASE="$(cd "$candidate" && pwd)"
        break
    fi
done
if [ -z "$ASSETS_BASE" ]; then
    echo "ERROR: assets/linux/agentmux.desktop not found near $SCRIPT_DIR" >&2
    echo "       checked: $SCRIPT_DIR/assets/linux, $SCRIPT_DIR/../assets/linux" >&2
    exit 1
fi
TEMPLATE="$ASSETS_BASE/agentmux.desktop"
ICON_SRC_ROOT="$ASSETS_BASE/icons/hicolor"

mkdir -p "$APPS_DIR"

# 1. Install raster icons
for size in 16 32 48 64 128 256 512; do
    src="$ICON_SRC_ROOT/${size}x${size}/apps/agentmux.png"
    dst="$ICONS_ROOT/${size}x${size}/apps/agentmux.png"
    if [ -f "$src" ]; then
        mkdir -p "$(dirname "$dst")"
        cp -f "$src" "$dst"
    fi
done

# 2. Install scalable SVG (for HiDPI / vector-aware launchers)
svg_src="$ICON_SRC_ROOT/scalable/apps/agentmux.svg"
if [ -f "$svg_src" ]; then
    mkdir -p "$ICONS_ROOT/scalable/apps"
    cp -f "$svg_src" "$ICONS_ROOT/scalable/apps/agentmux.svg"
fi

# 3. Render .desktop with Exec= substituted.
#
# Per the Desktop Entry Specification (§Exec): paths containing spaces or
# any reserved character — space, tab, newline, " ` $ \ < > ~ | & ; * ? # ( ) —
# must be enclosed in double quotes, with each ", `, $, \ inside the path
# backslash-escaped. AppImages frequently end up under paths with spaces
# (e.g. "~/My Apps/AgentMux.AppImage"); without quoting, launcher clicks
# silently fail because the parser splits the path into multiple arg tokens.
#
# Always quote (cheap and never wrong) and escape the four special chars.
# Use bash string substitution rather than sed for the template replacement
# so reserved path chars don't get re-interpreted as sed metacharacters.
escape_for_desktop_exec() {
    local s="$1"
    s="${s//\\/\\\\}"   # \ → \\
    s="${s//\"/\\\"}"   # " → \"
    s="${s//\$/\\\$}"   # $ → \$
    s="${s//\`/\\\`}"   # ` → \`
    printf '"%s"' "$s"
}
quoted_exec="$(escape_for_desktop_exec "$EXEC_PATH")"

desktop="$APPS_DIR/agentmux.desktop"
template_content="$(<"$TEMPLATE")"
printf '%s\n' "${template_content//__EXEC__/$quoted_exec}" > "$desktop"
chmod 644 "$desktop"

# 4. Refresh caches (best-effort; tools may be absent on minimal systems)
update-desktop-database "$APPS_DIR" 2>/dev/null || true
gtk-update-icon-cache -f "$ICONS_ROOT" 2>/dev/null || true

echo "✓ Installed $desktop with Exec=$EXEC_PATH"
