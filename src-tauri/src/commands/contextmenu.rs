// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Context menu command handler

use serde::{Deserialize, Serialize};
use tauri::menu::ContextMenu; // Trait providing .popup() method
use tauri::{AppHandle, Manager, Runtime};

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

#[tauri::command(rename_all = "camelCase")]
pub fn show_context_menu<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    menu: Option<Vec<MenuItem>>,
) -> Result<(), String> {
    tracing::debug!("show_context_menu workspace={} menu={:?}", workspace_id, menu.is_some());

    if menu.is_none() {
        return Ok(());
    }

    let menu_items = menu.unwrap();

    // Get the focused window
    let webview_window = app.get_webview_window("main")
        .ok_or_else(|| "Main window not found".to_string())?;
    let window = webview_window.as_ref().window();

    // Build Tauri menu from JSON structure
    let context_menu = build_menu_items(&menu_items, &app)
        .map_err(|e| format!("Failed to build menu: {}", e))?;

    // Show as popup at cursor position
    context_menu.popup(window.clone())
        .map_err(|e| format!("Failed to show context menu: {}", e))?;

    Ok(())
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
            Some("checkbox") => {
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
                // Regular menu item (submenus not supported in context menus for simplicity)
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

    builder.build()
}
