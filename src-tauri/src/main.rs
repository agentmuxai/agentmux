// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // NOTE: We previously forced GDK_BACKEND=x11 on Wayland so that GTK's
    // begin_move_drag worked for window dragging (via drag.rs). However, this caused
    // GTK to use the XIM input method bridge (via IBus's XIM server), which has a bug
    // that makes the first Backspace in a sequence insert a spurious character instead
    // of deleting. Since drag.rs already uses a direct GTK button-press-event signal
    // handler with event.time(), begin_move_drag works fine on the native Wayland backend
    // too — the GDK_BACKEND=x11 workaround is no longer needed.
    //
    // If window dragging breaks on Wayland, re-examine drag.rs and the
    // GDK_BACKEND=x11 approach; but keyboard input correctness takes priority.

    agentmux_lib::run()
}
