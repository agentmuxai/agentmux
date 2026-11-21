// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `workspace.importuserworkspace`: open a saved workspace file as a new workspace. Whether the file's terminal commands
//! run and its agents start is the person's answer in the import preview, so only the person's own window may call it
//! (an agent's connection, and any connection that is not the window, is refused). The HTTP service route no longer
//! imports (`service::user_workspace_files::handle_import`): it cannot tell the window from any other caller. The tests
//! are beside the import's own, in `service/user_workspace_files_tests.rs`.

use super::*;
use crate::backend::rpc_types::{CommandWorkspaceImportUserWorkspaceData, COMMAND_WORKSPACE_IMPORT_USER_WORKSPACE};

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_WORKSPACE_IMPORT_USER_WORKSPACE, move |req: CommandWorkspaceImportUserWorkspaceData, ctx| {
        let state = state.clone();
        async move {
            ctx.operator_only("only the person's own window imports a workspace: whether its commands run and its agents start is their answer in the preview")?;
            crate::server::service::user_workspace_files::import_for_window(&state, req).await
        }
    });
}
