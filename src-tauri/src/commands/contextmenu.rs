// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Context menu command handler

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::menu::ContextMenu; // Trait providing .popup() / .popup_at() methods
use tauri::{AppHandle, Manager, Runtime, WebviewWindow};
#[cfg(target_os = "linux")]
use tauri::LogicalPosition;

/// Tracks which window opened the most recent context menu so that
/// `handle_menu_event` in menu.rs can route the click callback back
/// to the correct window instead of guessing via `is_focused()`.
static CONTEXT_MENU_ORIGIN: Mutex<Option<String>> = Mutex::new(None);

/// Take (consume) the stored originating window label, if any.
pub fn take_context_menu_origin() -> Option<String> {
    CONTEXT_MENU_ORIGIN.lock().ok().and_then(|mut g| g.take())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuItem {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sublabel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    submenu: Option<Vec<MenuItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MenuPosition {
    x: f64,
    y: f64,
}

#[tauri::command(rename_all = "camelCase")]
pub fn show_context_menu<R: Runtime>(
    webview_window: WebviewWindow<R>,
    workspace_id: String,
    menu: Option<Vec<MenuItem>>,
    position: Option<MenuPosition>,
) -> Result<(), String> {
    tracing::debug!(
        "show_context_menu workspace={} window={} menu={:?}",
        workspace_id,
        webview_window.label(),
        menu.is_some()
    );

    if menu.is_none() {
        return Ok(());
    }

    let menu_items = menu.unwrap();
    let app = webview_window.app_handle();
    let window = webview_window.as_ref().window();

    // Store the originating window label so handle_menu_event can route
    // the context-menu-click event back to the correct window.
    if let Ok(mut guard) = CONTEXT_MENU_ORIGIN.lock() {
        *guard = Some(webview_window.label().to_string());
    }

    // Build Tauri menu from JSON structure
    let context_menu = build_menu_items(&menu_items, app)
        .map_err(|e| format!("Failed to build menu: {}", e))?;

    // On Linux/Wayland, popup() defaults to position (0,0) because the compositor
    // does not expose the cursor position to apps. Use popup_at() with the logical
    // coordinates from the mouse event instead.
    // On macOS and Windows, popup() already tracks the cursor correctly — leave
    // that path unchanged to avoid any platform-specific regressions.
    #[cfg(target_os = "linux")]
    {
        if let Some(pos) = position {
            context_menu.popup_at(window.clone(), LogicalPosition::new(pos.x, pos.y))
                .map_err(|e| format!("Failed to show context menu: {}", e))?;
        } else {
            context_menu.popup(window.clone())
                .map_err(|e| format!("Failed to show context menu: {}", e))?;
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = position; // unused on macOS/Windows
        context_menu.popup(window.clone())
            .map_err(|e| format!("Failed to show context menu: {}", e))?;
    }

    Ok(())
}

/// Recursively build a `Submenu` from a slice of `MenuItem`.
/// Used for nested submenus (e.g. the Opacity submenu).
fn build_submenu<R: Runtime>(
    label: &str,
    items: &[MenuItem],
    app: &AppHandle<R>,
) -> Result<tauri::menu::Submenu<R>, tauri::Error> {
    let mut builder = tauri::menu::SubmenuBuilder::new(app, label);
    for item in items {
        if let Some(false) = item.visible {
            continue;
        }
        match item.r#type.as_deref() {
            Some("separator") => {
                builder = builder.separator();
            }
            Some("checkbox") | Some("radio") => {
                let checked = item.checked.unwrap_or(false);
                let enabled = item.enabled.unwrap_or(true);
                let lbl = item.label.clone().unwrap_or_default();
                let id = item.id.clone();
                let check_item = tauri::menu::CheckMenuItemBuilder::new(&lbl)
                    .id(&id)
                    .checked(checked)
                    .enabled(enabled)
                    .build(app)?;
                builder = builder.item(&check_item);
            }
            _ => {
                if let Some(sub_items) = &item.submenu {
                    let lbl = item.label.clone().unwrap_or_default();
                    let submenu = build_submenu(&lbl, sub_items, app)?;
                    builder = builder.item(&submenu);
                } else {
                    let enabled = item.enabled.unwrap_or(true);
                    let lbl = item.label.clone().unwrap_or_default();
                    let id = item.id.clone();
                    let menu_item = tauri::menu::MenuItemBuilder::new(&lbl)
                        .id(&id)
                        .enabled(enabled)
                        .build(app)?;
                    builder = builder.item(&menu_item);
                }
            }
        }
    }
    builder.build()
}

fn build_menu_items<R: Runtime>(
    items: &[MenuItem],
    app: &AppHandle<R>,
) -> Result<tauri::menu::Menu<R>, tauri::Error> {
    let mut builder = tauri::menu::MenuBuilder::new(app);
    for item in items {
        // Handle visibility
        if let Some(false) = item.visible {
            continue;
        }

        // Handle different menu item types
        match item.r#type.as_deref() {
            Some("separator") => {
                builder = builder.separator();
            }
            Some("checkbox") | Some("radio") => {
                let checked = item.checked.unwrap_or(false);
                let enabled = item.enabled.unwrap_or(true);
                let label = item.label.clone().unwrap_or_default();
                let id = item.id.clone();

                let check_item = tauri::menu::CheckMenuItemBuilder::new(&label)
                    .id(&id)
                    .checked(checked)
                    .enabled(enabled)
                    .build(app)?;

                builder = builder.item(&check_item);
            }
            _ => {
                if let Some(sub_items) = &item.submenu {
                    let label = item.label.clone().unwrap_or_default();
                    let submenu = build_submenu(&label, sub_items, app)?;
                    builder = builder.item(&submenu);
                } else {
                    let enabled = item.enabled.unwrap_or(true);
                    let label = item.label.clone().unwrap_or_default();
                    let id = item.id.clone();

                    let menu_item = tauri::menu::MenuItemBuilder::new(&label)
                        .id(&id)
                        .enabled(enabled)
                        .build(app)?;

                    builder = builder.item(&menu_item);
                }
            }
        }
    }

    builder.build()
}
