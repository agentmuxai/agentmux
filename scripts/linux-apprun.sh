#!/usr/bin/env bash
# AppImage AppRun for AgentMux on Linux (CEF runtime).
#
# Why this is so short:
#   Earlier versions set WEBKIT_DISABLE_DMABUF_RENDERER, XMODIFIERS,
#   GTK_IM_MODULE, and GDK_BACKEND. Those were workarounds for the
#   Tauri/WebKitGTK era. None of them apply to the current CEF/Chromium
#   build — CEF doesn't read WebKit env vars, has its own IME path
#   (InputMethodAuralinux), and uses Ozone (not GDK) for Wayland/X11.
#   Carrying them forward biased CEF toward stale Tauri-era behavior.
#   Audit + rationale: docs/specs/linux-cef-flags-audit-2026-05-08.md.
#
# Icon / desktop registration:
#   The agentmux-cef binary sets xdg_toplevel.app_id="agentmux" via
#   WindowDelegate::linux_window_properties (agentmux-cef/src/app.rs).
#   This script registers a matching ~/.local/share/applications/agentmux.desktop
#   so window managers can show the AgentMux logo. Registration is
#   delegated to install-linux-desktop.sh — also used by `task dev` and
#   portable-bundle installs, so all three run modes share one source of
#   truth.
set -e
this_dir="$(readlink -f "$(dirname "$0")")"
export APPDIR="$this_dir"
if [ -n "$APPIMAGE" ] && [ -x "$this_dir/install-linux-desktop.sh" ]; then
    bash "$this_dir/install-linux-desktop.sh" "$APPIMAGE" || true
fi
# libcef.so + EGL/GLESv2 sit in usr/bin alongside agentmux-cef per CEF's
# colocation convention. The binary is built without RPATH, so the
# dynamic linker needs LD_LIBRARY_PATH to find them.
export LD_LIBRARY_PATH="$this_dir/usr/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$this_dir/usr/bin/agentmux" "$@"
