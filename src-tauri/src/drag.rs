// Linux-only native GTK drag handler for window header dragging.
// On Wayland (forced to XWayland via GDK_BACKEND=x11), button-press-event
// fires on the WebView and we use event.time() with begin_move_drag so that
// the X11 timestamp is correct. Must be called for every new window.

#[cfg(target_os = "linux")]
pub fn attach_drag_handler<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    use gtk::prelude::*;

    window
        .with_webview(|webview| {
            let webview = webview.inner();

            // Walk up to find the gtk::Window (WebView → GtkBox → GtkWindow)
            let gtk_win = webview
                .parent()
                .and_then(|p| p.parent())
                .and_then(|w| w.dynamic_cast::<gtk::Window>().ok());

            // Must explicitly enable BUTTON_PRESS events on the webview widget.
            // Without this, GTK never delivers button-press-event to signal handlers.
            webview.add_events(
                gtk::gdk::EventMask::BUTTON_PRESS_MASK
                    | gtk::gdk::EventMask::BUTTON1_MOTION_MASK,
            );

            // Header height in logical pixels (matches window-header.scss)
            const HEADER_HEIGHT: f64 = 40.0;

            // Connect to the WebView's button-press-event.
            // Using event.time() preserves the X11 timestamp required for
            // begin_move_drag to work reliably via XWayland.
            webview.connect_button_press_event(move |_wv, event| {
                let (_, y) = event.position();
                if event.button() != 1 {
                    return glib::Propagation::Proceed;
                }
                if y > HEADER_HEIGHT {
                    return glib::Propagation::Proceed;
                }
                if let Some(win) = gtk_win.as_ref() {
                    let (root_x, root_y) = event.root();
                    win.begin_move_drag(1, root_x as i32, root_y as i32, event.time());
                }
                // Propagate so header buttons still receive clicks
                glib::Propagation::Proceed
            });

            tracing::info!("[linux-drag] Native GTK drag handler attached to webview");
        })
        .ok();
}
