#!/usr/bin/env bash
# AppImage AppRun script for AgentMux on Linux.
# Replaces linuxdeploy's default AppRun which forces GDK_BACKEND=x11 and
# does not set WEBKIT_DISABLE_DMABUF_RENDERER=1.
#
# Icon / desktop registration:
#   The agentmux-cef binary sets xdg_toplevel.app_id="agentmux" via
#   WindowDelegate::linux_window_properties (agentmux-cef/src/app.rs).
#   This script registers a matching ~/.local/share/applications/agentmux.desktop
#   so window managers can show the AgentMux logo. Registration is delegated
#   to install-linux-desktop.sh — the same script used by `task dev` and
#   portable-bundle installs, so all three run modes share one source of truth.
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
if [ -n "$APPIMAGE" ] && [ -x "$this_dir/install-linux-desktop.sh" ]; then
    bash "$this_dir/install-linux-desktop.sh" "$APPIMAGE" || true
fi
# libcef.so + EGL/GLESv2 live in usr/bin alongside agentmux-cef per CEF's
# colocation convention. The binary is built without RPATH, so the dynamic
# linker needs LD_LIBRARY_PATH to find them. Also set CEF_LIBRARY_PATH for
# CEF-internal probes.
export LD_LIBRARY_PATH="$this_dir/usr/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$this_dir/usr/bin/agentmux" "$@"
