// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Application menu builder for Tauri.
// Replaces emain/menu.ts from the Electron build.

#![allow(dead_code)] // Menu builders disabled for frameless window build, see lib.rs

use tauri::menu::*;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Build the application menu for AgentMux Corp.
/// This creates the same menu structure as the Electron version:
/// - App/File/Edit/View/Workspace/Window menus
pub fn build_app_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;

    // App menu (macOS) / File menu (Windows/Linux)
    let app_menu = build_app_submenu(app)?;
    menu.append(&app_menu)?;

    // File menu
    let file_menu = build_file_submenu(app)?;
    menu.append(&file_menu)?;

    // Edit menu
    let edit_menu = build_edit_submenu(app)?;
    menu.append(&edit_menu)?;

    // View menu
    let view_menu = build_view_submenu(app)?;
    menu.append(&view_menu)?;

    // Workspace menu (dynamic, populated from backend)
    let workspace_menu = build_workspace_submenu(app)?;
    menu.append(&workspace_menu)?;

    // Window menu
    let window_menu = build_window_submenu(app)?;
    menu.append(&window_menu)?;

    Ok(menu)
}

fn build_app_submenu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let submenu = Submenu::new(app, "AgentMux", true)?;

    let about = MenuItem::with_id(app, "about", "About AgentMux", true, None::<&str>)?;
    submenu.append(&about)?;

    let check_updates = MenuItem::with_id(app, "check-updates", "Check for Updates", true, None::<&str>)?;
    submenu.append(&check_updates)?;

    submenu.append(&PredefinedMenuItem::separator(app)?)?;

    #[cfg(target_os = "macos")]
    {
        submenu.append(&PredefinedMenuItem::services(app, None)?)?;
        submenu.append(&PredefinedMenuItem::separator(app)?)?;
        submenu.append(&PredefinedMenuItem::hide(app, None)?)?;
        submenu.append(&PredefinedMenuItem::hide_others(app, None)?)?;
        submenu.append(&PredefinedMenuItem::separator(app)?)?;
    }

    submenu.append(&PredefinedMenuItem::quit(app, None)?)?;

    Ok(submenu)
}

fn build_file_submenu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let submenu = Submenu::new(app, "File", true)?;

    let new_window = MenuItem::with_id(
        app,
        "new-window",
        "New Window",
        true,
        Some("CommandOrControl+Shift+N"),
    )?;
    submenu.append(&new_window)?;

    let close = MenuItem::with_id(app, "close-window", "Close", true, None::<&str>)?;
    submenu.append(&close)?;

    Ok(submenu)
}

fn build_edit_submenu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let submenu = Submenu::new(app, "Edit", true)?;

    #[cfg(target_os = "macos")]
    {
        submenu.append(&PredefinedMenuItem::undo(app, Some("Command+Z"))?)?;
        submenu.append(&PredefinedMenuItem::redo(app, Some("Command+Shift+Z"))?)?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        submenu.append(&PredefinedMenuItem::undo(app, None::<&str>)?)?;
        submenu.append(&PredefinedMenuItem::redo(app, None::<&str>)?)?;
    }
    submenu.append(&PredefinedMenuItem::separator(app)?)?;
    submenu.append(&PredefinedMenuItem::cut(app, None)?)?;
    submenu.append(&PredefinedMenuItem::copy(app, None)?)?;
    submenu.append(&PredefinedMenuItem::paste(app, None)?)?;
    submenu.append(&PredefinedMenuItem::separator(app)?)?;
    submenu.append(&PredefinedMenuItem::select_all(app, None)?)?;

    Ok(submenu)
}

fn build_view_submenu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let submenu = Submenu::new(app, "View", true)?;

    let reload = MenuItem::with_id(
        app,
        "reload-tab",
        "Reload Tab",
        true,
        Some("Shift+CommandOrControl+R"),
    )?;
    submenu.append(&reload)?;

    let relaunch = MenuItem::with_id(app, "relaunch-windows", "Relaunch All Windows", true, None::<&str>)?;
    submenu.append(&relaunch)?;

    let clear_cache = MenuItem::with_id(app, "clear-cache", "Clear Tab Cache", true, None::<&str>)?;
    submenu.append(&clear_cache)?;

    #[cfg(target_os = "macos")]
    let devtools_accel = "Option+Command+I";
    #[cfg(not(target_os = "macos"))]
    let devtools_accel = "Alt+Shift+I";

    let devtools = MenuItem::with_id(app, "toggle-devtools", "Toggle DevTools", true, Some(devtools_accel))?;
    submenu.append(&devtools)?;

    submenu.append(&PredefinedMenuItem::separator(app)?)?;

    let zoom_reset = MenuItem::with_id(app, "zoom-reset", "Reset Zoom", true, Some("CommandOrControl+0"))?;
    submenu.append(&zoom_reset)?;

    let zoom_in = MenuItem::with_id(app, "zoom-in", "Zoom In", true, Some("CommandOrControl+="))?;
    submenu.append(&zoom_in)?;

    let zoom_out = MenuItem::with_id(app, "zoom-out", "Zoom Out", true, Some("CommandOrControl+-"))?;
    submenu.append(&zoom_out)?;

    submenu.append(&PredefinedMenuItem::separator(app)?)?;
    submenu.append(&PredefinedMenuItem::fullscreen(app, None)?)?;

    Ok(submenu)
}

fn build_workspace_submenu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let submenu = Submenu::new(app, "Workspace", true)?;

    let create_workspace = MenuItem::with_id(app, "create-workspace", "Create Workspace", true, None::<&str>)?;
    submenu.append(&create_workspace)?;

    submenu.append(&PredefinedMenuItem::separator(app)?)?;

    // TODO: Dynamically populate workspace list from backend
    // This will require listening to workspace updates and rebuilding the menu

    Ok(submenu)
}

fn build_window_submenu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let submenu = Submenu::new(app, "Window", true)?;

    submenu.append(&PredefinedMenuItem::minimize(app, None)?)?;
    #[cfg(target_os = "macos")]
    {
        submenu.append(&PredefinedMenuItem::separator(app)?)?;
        // Note: macOS will automatically add window list here
    }

    Ok(submenu)
}

/// Handle menu item clicks
pub fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    // Get the focused window instead of hard-coding "main"
    // This ensures menu actions work correctly in multi-window scenarios
    let window = app.webview_windows()
        .values()
        .find(|w| w.is_focused().unwrap_or(false))
        .cloned()
        .or_else(|| app.get_webview_window("main"));

    match event.id.as_ref() {
        "about" => {
            if let Some(w) = window {
                let _ = w.emit("menu-item-about", ());
            }
        }
        "check-updates" => {
            tracing::info!("Check for updates clicked");
            // TODO: Implement update checker
        }
        "new-window" => {
            tracing::info!("New window requested via menu");
            let app_handle = app.clone();
            tauri::async_runtime::spawn(async move {
                match crate::commands::window::open_new_window(app_handle).await {
                    Ok(label) => tracing::info!("Successfully created new window: {}", label),
                    Err(e) => tracing::error!("Failed to create new window: {}", e),
                }
            });
        }
        "close-window" => {
            if let Some(w) = window {
                let _ = w.close();
            }
        }
        "reload-tab" => {
            if let Some(w) = window {
                let _ = w.eval("location.reload()");
            }
        }
        "relaunch-windows" => {
            tracing::info!("Relaunch all windows requested");
            // TODO: Implement window relaunch
        }
        "clear-cache" => {
            tracing::info!("Clear cache requested");
            // TODO: Implement cache clearing
        }
        "toggle-devtools" => {
            if let Some(w) = window {
                if w.is_devtools_open() {
                    tracing::info!("Closing devtools for window: {}", w.label());
                    let _ = w.close_devtools();
                } else {
                    tracing::info!("Opening devtools for window: {}", w.label());
                    let _ = w.open_devtools();
                }
            }
        }
        "zoom-reset" => {
            if let Some(w) = window {
                if let Err(e) = crate::commands::window::set_zoom_factor(app.state(), w.clone(), 1.0) {
                    tracing::error!("Failed to reset zoom: {}", e);
                } else {
                    tracing::debug!("Reset zoom to 1.0");
                }
            }
        }
        "zoom-in" => {
            if let Some(w) = window {
                let current = crate::commands::window::get_zoom_factor(app.state());
                let new_factor = (current + 0.2).min(3.0);
                if let Err(e) = crate::commands::window::set_zoom_factor(app.state(), w.clone(), new_factor) {
                    tracing::error!("Failed to zoom in: {}", e);
                } else {
                    tracing::debug!("Zoomed in to {:.1}", new_factor);
                }
            }
        }
        "zoom-out" => {
            if let Some(w) = window {
                let current = crate::commands::window::get_zoom_factor(app.state());
                let new_factor = (current - 0.2).max(0.5);
                if let Err(e) = crate::commands::window::set_zoom_factor(app.state(), w.clone(), new_factor) {
                    tracing::error!("Failed to zoom out: {}", e);
                } else {
                    tracing::debug!("Zoomed out to {:.1}", new_factor);
                }
            }
        }
        "create-workspace" => {
            tracing::info!("Create workspace requested");
            // TODO: Call workspace service
        }
        _ => {
            // Handle context menu clicks - emit to frontend
            tracing::debug!("Context menu click: {}", event.id.as_ref());
            if let Some(w) = window {
                let _ = w.emit("context-menu-click", event.id.as_ref());
            }
        }
    }
}
