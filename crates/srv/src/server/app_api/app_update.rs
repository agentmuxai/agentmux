// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app.update-status`: what srv knows about newer AgentMux releases
//! (`backend::update_check`), for a window that opens after the last
//! `app:updatestatus` event. `app.update-download`: download and stage the
//! release it found (`backend::update_download`); progress follows as events.
//! `app.upgrade-restart` / `app.upgrade-cancel`: restart into it once agents
//! are idle, or stop waiting (`backend::update_restart`).

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_typed(COMMAND_APP_UPDATE_STATUS, move |_req: Option<NoArgsReq>, _ctx| async move {
        Ok::<_, String>(crate::backend::update_check::current_status())
    });
    // The user's click only: an agent asks through RequestUpgrade, which
    // needs the user's say-so (the spec's §5.1).
    engine.register_typed(COMMAND_APP_UPDATE_DOWNLOAD, move |_req: Option<NoArgsReq>, ctx| async move {
        if !ctx.agent_id.is_empty() {
            return Err("FORBIDDEN: only the user starts downloading an AgentMux release".to_string());
        }
        crate::backend::update_download::start()
    });
    engine.register_typed(
        COMMAND_APP_UPGRADE_RESTART,
        move |req: crate::backend::update_restart::CommandAppUpgradeRestartData, ctx| async move {
            if !ctx.agent_id.is_empty() {
                return Err("FORBIDDEN: only the user restarts AgentMux to upgrade it".to_string());
            }
            crate::backend::update_restart::start(req.now)
        },
    );
    engine.register_typed(COMMAND_APP_UPGRADE_CANCEL, move |_req: Option<NoArgsReq>, _ctx| async move {
        Ok::<_, String>(crate::backend::update_restart::cancel())
    });
}
