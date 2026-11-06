// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `object` service handler (GetObject, CreateBlock, UpdateObject, …).

use crate::backend::service::{WebCallType, WebReturnType};

use super::super::AppState;

#[path = "object_methods.rs"]
mod methods;

pub(super) async fn handle_object_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args = &call.args;
    let reply = match call.method.as_str() {
        "GetObject" => methods::get_object(state, args),
        "GetObjects" => methods::get_objects(state, args),
        "CreateBlock" => methods::create_block(state, call).await,
        "DeleteBlock" => methods::delete_block(state, args).await,
        "QuitAgent" => methods::quit_agent(state, args).await,
        "ShutdownAgent" => methods::shutdown_agent(state, args).await,
        "ShutdownPreview" => methods::shutdown_preview(state, args).await,
        "ClosePane" => methods::close_pane(state, args).await,
        "ApplyLayoutIntent" => methods::apply_layout_intent(state, args).await,
        "UpdateObject" => methods::update_object_row(state, args),
        "UpdateObjectMeta" => methods::update_object_meta_patch(state, args).await,
        "UpdateTabName" => methods::update_tab_name(state, args).await,
        _ => Err(WebReturnType::error(format!("unknown object method: {}", call.method))),
    };
    match reply {
        Ok(reply) | Err(reply) => reply,
    }
}
