// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app.update-status`: what srv knows about newer AgentMux releases
//! (`backend::update_check`), for a window that opens after the last
//! `app:updatestatus` event. `app.update-download`: download and stage the
//! release it found (`backend::update_download`); progress follows as events.

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_typed(COMMAND_APP_UPDATE_STATUS, move |_req: Option<NoArgsReq>, _ctx| async move {
        Ok::<_, String>(crate::backend::update_check::current_status())
    });
    engine.register_typed(COMMAND_APP_UPDATE_DOWNLOAD, move |_req: Option<NoArgsReq>, _ctx| async move {
        crate::backend::update_download::start()
    });
}
