// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reopening the last session's other windows where they were
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §6.1; srv
//! `session_windows.rs` rebuilds their workspaces, the main window's frontend
//! asks for each one here).

use std::sync::Arc;

use serde_json::json;

use crate::state::AppState;

/// A saved window rect, `(x, y, width, height)` in physical pixels, when the
/// request carries a whole one with a real size.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn saved_rect(args: &serde_json::Value) -> Option<(i32, i32, i32, i32)> {
    let get = |k: &str| args.get(k).and_then(|v| v.as_i64()).and_then(|v| i32::try_from(v).ok());
    let rect = (get("x")?, get("y")?, get("width")?, get("height")?);
    (rect.2 > 0 && rect.3 > 0).then_some(rect)
}

/// Whether the physical point `(x, y)` is on a monitor that is connected now.
#[cfg(target_os = "windows")]
fn on_a_monitor(x: i32, y: i32) -> bool {
    use windows_sys::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONULL};
    let pt = windows_sys::Win32::Foundation::POINT { x, y };
    !unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONULL) }.is_null()
}

/// `open_restored_window`: open a window onto `workspace_id`, a workspace a
/// session restore rebuilt, at its saved position and size (`x`, `y`, `width`,
/// `height`). Those come from srv's window row as `GetWindowRect` reported
/// them, in physical pixels; a new window is placed in DIP, so they are scaled
/// by the monitor the window was on. When that spot is on no monitor now (one
/// unplugged since), or there is no saved size, or off Windows, the window
/// opens where a new window goes.
pub fn open_restored_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let workspace_id = args
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .filter(|ws| super::creation::is_object_id(ws))
        .ok_or("open_restored_window: invalid workspace_id")?;
    #[cfg(target_os = "windows")]
    if let Some((x, y, w, h)) = saved_rect(args).filter(|&(x, y, _, _)| on_a_monitor(x, y)) {
        let scale = crate::app::dpi_scale_at(x, y);
        let dip = |v: i32| (v as f32 / scale).round() as i64;
        tracing::info!(workspace_id, x, y, w, h, scale, "[window] reopening a restored window where it was");
        return crate::commands::drag::open_window_at_position(
            state,
            &json!({
                "workspaceId": workspace_id,
                "width": dip(w),
                "height": dip(h),
                "tabAnchorX": dip(x),
                "tabAnchorY": dip(y),
            }),
        );
    }
    super::open_new_window(state, &json!({ "workspace_id": workspace_id }))
}

#[cfg(test)]
mod tests {
    use super::saved_rect;
    use serde_json::json;

    #[test]
    fn only_a_whole_rect_with_a_size_is_used() {
        assert_eq!(saved_rect(&json!({ "x": -1920, "y": 40, "width": 1200, "height": 800 })), Some((-1920, 40, 1200, 800)));
        assert_eq!(saved_rect(&json!({ "x": 0, "y": 0, "width": 0, "height": 0 })), None, "a window row never placed");
        assert_eq!(saved_rect(&json!({ "x": 10, "y": 20 })), None);
        assert_eq!(saved_rect(&json!({ "x": 10, "y": 20, "width": 1e12, "height": 800 })), None);
    }
}
