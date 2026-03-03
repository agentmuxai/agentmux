use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

/// Build the system tray icon and menu
pub fn build_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    // Build tray menu
    let show_hide = MenuItem::with_id(app, "show_hide", "Show/Hide", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit AgentMux", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show_hide, &quit])?;

    // Build tray icon
    let icon = app
        .default_window_icon()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "No default window icon configured"))?;

    let _tray = TrayIconBuilder::new()
        .icon(icon.clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show_hide" => {
                toggle_window_visibility(app);
            }
            "quit" => {
                tracing::info!("Quit requested from tray menu");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                toggle_window_visibility(app);
            }
        })
        .build(app)?;

    Ok(())
}

/// Toggle the main window visibility
fn toggle_window_visibility<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        match window.is_visible() {
            Ok(true) => {
                // Window is visible → hide it
                if let Err(e) = window.hide() {
                    tracing::error!("Failed to hide window: {}", e);
                }
            }
            Ok(false) => {
                // Window is hidden → show and focus it
                if let Err(e) = window.show() {
                    tracing::error!("Failed to show window: {}", e);
                }
                if let Err(e) = window.set_focus() {
                    tracing::error!("Failed to focus window: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to check window visibility: {}", e);
            }
        }
    }
}

/// Update tray menu item labels dynamically
pub fn update_tray_menu<R: Runtime>(app: &AppHandle<R>, window_visible: bool) {
    // Future enhancement: Update "Show/Hide" label to "Show" or "Hide" based on state
    // This requires storing a reference to the tray menu items
    let _ = (app, window_visible); // Suppress unused warnings for now
}
