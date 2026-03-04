// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // On Linux/Wayland: force X11 (XWayland) so window dragging works.
    // Wayland's xdg_toplevel_move requires a compositor event serial that is not
    // available through the async Tauri IPC path, so dragging never works natively
    // on Wayland. GDK_BACKEND=x11 uses XWayland where X11's timestamp=0 works.
    // WEBKIT_DISABLE_DMABUF_RENDERER=1 prevents the blank-screen issue caused by
    // WebKit's DMA-BUF GPU renderer failing when GTK switches to X11 backend.
    #[cfg(target_os = "linux")]
    if std::env::var("GDK_BACKEND").is_err() && std::env::var("WAYLAND_DISPLAY").is_ok() {
        std::env::set_var("GDK_BACKEND", "x11");
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    agentmux_lib::run()
}
