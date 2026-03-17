#!/usr/bin/env bash
# AppImage AppRun script for AgentMux on Linux.
# Replaces linuxdeploy's default AppRun which forces GDK_BACKEND=x11 and
# does not set WEBKIT_DISABLE_DMABUF_RENDERER=1.
#
# Icon / desktop registration:
#   On both X11 and Wayland, GNOME matches windows to .desktop files via
#   xdg_toplevel.app_id (Wayland) or WM_CLASS (X11), both of which equal
#   the binary name "agentmux".  We register agentmux.desktop only.
set -e
this_dir="$(readlink -f "$(dirname "$0")")"
export APPDIR="$this_dir"
export WEBKIT_DISABLE_DMABUF_RENDERER=1
# Use native Wayland when available; fall back to X11 for pure X11 sessions.
if [ -n "$WAYLAND_DISPLAY" ]; then
    export GDK_BACKEND=wayland
else
    export GDK_BACKEND=x11
fi
export XMODIFIERS=""
export GTK_IM_MODULE=gtk-im-context-simple
if [ -n "$APPIMAGE" ]; then
    _icon_dir="$HOME/.local/share/icons/hicolor/256x256/apps"
    _apps_dir="$HOME/.local/share/applications"
    mkdir -p "$_icon_dir" "$_apps_dir"
    cp -f "$this_dir/AgentMux.png" "$_icon_dir/agentmux.png"
    _cur_exec=$(grep -m1 "^Exec=" "$_apps_dir/agentmux.desktop" 2>/dev/null | cut -d= -f2- || true)
    _cur_icon=$(grep -m1 "^Icon=" "$_apps_dir/agentmux.desktop" 2>/dev/null | cut -d= -f2- || true)
    # Update if Exec= is stale OR if Icon= is an absolute path (old broken format).
    if [ "$_cur_exec" != "$APPIMAGE" ] || echo "$_cur_icon" | grep -q "^/"; then
        _content=$(sed "s|^Exec=.*|Exec=$APPIMAGE|" "$this_dir/AgentMux.desktop")
        printf '%s\n' "$_content" > "$_apps_dir/agentmux.desktop"
        # Ensure hicolor has an index.theme so gtk-update-icon-cache succeeds
        if [ ! -f "$HOME/.local/share/icons/hicolor/index.theme" ]; then
            cp /usr/share/icons/hicolor/index.theme "$HOME/.local/share/icons/hicolor/index.theme" 2>/dev/null || true
        fi
        update-desktop-database "$_apps_dir" 2>/dev/null || true
        gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
    fi
fi
exec "$this_dir/usr/bin/agentmux" "$@"
