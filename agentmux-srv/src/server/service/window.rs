// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `window` service handler (GetWindow, CreateWindow, CloseWindow, …).
//!
//! The dispatcher below just routes each method to its handler; the
//! handler bodies live in sibling modules:
//! * [`super::window_query`] — `GetWindow` / `FindWindowByLabel`.
//! * [`super::window_create`] — `CreateWindow`.
//! * [`super::window_close`] — `CloseWindow`.
//! * [`super::window_mutate`] — `SwitchWorkspace` / `SetWindowPosAndSize` /
//!   `SetWindowOpacity` / `SetWindowTopology`.

use crate::backend::service::{WebCallType, WebReturnType};

use super::super::AppState;
use super::window_close::handle_close_window;
use super::window_create::handle_create_window;
use super::window_mutate::{
    handle_set_window_opacity, handle_set_window_pos_and_size, handle_set_window_topology,
    handle_switch_workspace,
};
use super::window_query::{handle_find_window_by_label, handle_get_window};

pub(super) async fn handle_window_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    match call.method.as_str() {
        "GetWindow" => handle_get_window(state, call).await,
        "FindWindowByLabel" => handle_find_window_by_label(state, call).await,
        "CreateWindow" => handle_create_window(state, call).await,
        "CloseWindow" => handle_close_window(state, call).await,
        "SwitchWorkspace" => handle_switch_workspace(state, call).await,
        "SetWindowPosAndSize" => handle_set_window_pos_and_size(state, call).await,
        "SetWindowOpacity" => handle_set_window_opacity(state, call).await,
        "SetWindowTopology" => handle_set_window_topology(state, call).await,
        _ => WebReturnType::error(format!("unknown window method: {}", call.method)),
    }
}
