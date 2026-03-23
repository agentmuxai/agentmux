// Linux-only native GTK drag handler for window header dragging.
//
// On native Wayland, begin_move_drag must NOT be called on button-press.
// The compositor immediately grabs the pointer, so the button release never
// reaches WebKit — all header buttons become unclickable. On X11/XWayland
// this wasn't a problem because X11's grab is cooperative and still delivers
// the full press→release cycle to the application.
//
// Fix: detect drag intent via pointer motion. On button-press we record the
// position; on motion we check if the pointer moved beyond DRAG_THRESHOLD
// pixels and only then call begin_move_drag. Simple clicks (no significant
// movement) are never intercepted and WebKit receives them normally.
//
// JS_DRAG_ACTIVE flag: set by the `set_js_drag_active` Tauri command when a
// JS-level drag (tab or pane) is in progress. The motion handler will NOT
// call begin_move_drag while this flag is true, preventing the GTK window-drag
// from firing mid-drag and grabbing the Wayland compositor pointer.

/// Set to true while a JS-level drag (tab or pane via pragmatic-dnd) is active.
/// Checked by the GTK motion handler to suppress `begin_move_drag`.
#[cfg(target_os = "linux")]
pub static JS_DRAG_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "linux")]
pub fn attach_drag_handler<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    use gtk::prelude::*;
    use std::cell::Cell;
    use std::rc::Rc;

    window
        .with_webview(|webview| {
            let webview = webview.inner();

            // Walk up to find the gtk::Window (WebView → GtkBox → GtkWindow)
            let gtk_win = webview
                .parent()
                .and_then(|p| p.parent())
                .and_then(|w| w.dynamic_cast::<gtk::Window>().ok());

            webview.add_events(
                gtk::gdk::EventMask::BUTTON_PRESS_MASK
                    | gtk::gdk::EventMask::BUTTON_RELEASE_MASK
                    | gtk::gdk::EventMask::BUTTON1_MOTION_MASK,
            );

            // Header height in logical pixels. CSS base is 33px but grows at
            // low zoom via max(33px, 33px * zoomfactor-inv). 42px covers ~0.8x zoom.
            const HEADER_HEIGHT: f64 = 42.0;
            // Minimum pointer movement (pixels) before treating press-move as a drag.
            // 12px (was 4px) gives the async JS set_js_drag_active IPC call time to
            // complete before this threshold is reached, preventing begin_move_drag
            // from firing mid-tab-drag.
            const DRAG_THRESHOLD: f64 = 12.0;

            // Pending drag state: (root_x, root_y, timestamp) of the press event,
            // or None if no drag is pending.
            let drag_start: Rc<Cell<Option<(i32, i32, u32)>>> = Rc::new(Cell::new(None));
            let drag_start_motion = drag_start.clone();
            let drag_start_release = drag_start.clone();

            // On press in header: record position, but don't begin_move_drag yet.
            webview.connect_button_press_event(move |_wv, event| {
                let (_, y) = event.position();
                if event.button() == 1 && y <= HEADER_HEIGHT {
                    let (root_x, root_y) = event.root();
                    drag_start.set(Some((root_x as i32, root_y as i32, event.time())));
                }
                glib::Propagation::Proceed
            });

            // On motion: if moved beyond threshold AND no JS drag is active, begin the window move.
            let gtk_win_motion = gtk_win.clone();
            webview.connect_motion_notify_event(move |_wv, event| {
                if let Some((start_x, start_y, time)) = drag_start_motion.get() {
                    // Abort window-drag if a JS-level drag (tab/pane) is in progress.
                    if JS_DRAG_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
                        drag_start_motion.set(None);
                        return glib::Propagation::Proceed;
                    }
                    let (root_x, root_y) = event.root();
                    let dx = root_x - start_x as f64;
                    let dy = root_y - start_y as f64;
                    if dx * dx + dy * dy > DRAG_THRESHOLD * DRAG_THRESHOLD {
                        drag_start_motion.set(None);
                        if let Some(win) = gtk_win_motion.as_ref() {
                            win.begin_move_drag(1, root_x as i32, root_y as i32, time);
                        }
                    }
                }
                glib::Propagation::Proceed
            });

            // On release: cancel any pending drag (simple click, no motion).
            webview.connect_button_release_event(move |_wv, event| {
                if event.button() == 1 {
                    drag_start_release.set(None);
                }
                glib::Propagation::Proceed
            });

            tracing::info!("[linux-drag] Native GTK drag handler attached to webview");
        })
        .ok();
}
