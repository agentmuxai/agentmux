// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! RPC surface of the notification Router —
//! `docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §3.3.
//!
//! - `notify.emit`   renderer → router: a pane event the renderer is the
//!                   authority for (turn outcome, waiting-for-input).
//! - `notify.focus`  renderer → router: this window's focus state.
//! - `notify.ack`    presenter → router: user clicked / dismissed a toast.
//! - `notify.test`   Settings "Send test notification".
//! - `notify.takeactivation` new window → router: a click that arrived while
//!                   no window was open.
//!
//! `conn_id` is captured per connection so focus reports are keyed by window
//! and dropped on disconnect (see `websocket.rs`'s disconnect path).

use std::sync::Arc;

use crate::backend::notify::policy::{Family, FocusReport, NotifyKind};
use crate::backend::notify::router;
use crate::backend::rpc::engine::WshRpcEngine;

use super::AppState;

/// Pane-level events the renderer reports. `question` accompanies
/// `input_waiting` only; the router redacts it (§9.2).
#[derive(serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub enum NotifyPaneEvent {
    TurnStarted,
    TurnCompleted,
    TurnErrored,
    InputWaiting,
    InputResolved,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyEmitParams {
    pub block_id: String,
    pub event: NotifyPaneEvent,
    #[ts(optional)]
    pub question: Option<String>,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyFocusParams {
    pub window_focused: bool,
    #[ts(optional)]
    pub block_id: Option<String>,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyAckParams {
    pub id: String,
    pub clicked: bool,
}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyAckResult {
    /// False when no frontend window is connected — the caller (launcher)
    /// should open one; it will pick the block up via `notify.takeactivation`.
    pub has_window: bool,
}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyTakeActivationResult {
    #[ts(optional)]
    pub block_id: Option<String>,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyNoArgs {}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyOk {
    pub ok: bool,
}

fn router_for(state: &AppState) -> Arc<router::Router> {
    router::init(state.broker.clone(), state.config_watcher.clone(), state.mstore.clone())
}

pub fn register_notify_handlers(engine: &Arc<WshRpcEngine>, state: &AppState, conn_id: String) {
    let r = router_for(state);

    let rr = r.clone();
    engine.register_typed("notify.emit", move |p: NotifyEmitParams, _ctx| {
        let r = rr.clone();
        async move {
            if p.block_id.is_empty() {
                return Err("notify.emit: block_id required".to_string());
            }
            match p.event {
                NotifyPaneEvent::TurnStarted => r.resolve(&p.block_id, Family::Turn),
                NotifyPaneEvent::TurnCompleted => r.emit(NotifyKind::TurnCompleted, &p.block_id, None),
                NotifyPaneEvent::TurnErrored => r.emit(NotifyKind::TurnErrored, &p.block_id, None),
                NotifyPaneEvent::InputWaiting => r.emit(NotifyKind::InputWaiting, &p.block_id, p.question.as_deref()),
                NotifyPaneEvent::InputResolved => r.resolve(&p.block_id, Family::Input),
            }
            Ok(NotifyOk { ok: true })
        }
    });

    let rr = r.clone();
    let conn = conn_id.clone();
    engine.register_typed("notify.focus", move |p: NotifyFocusParams, _ctx| {
        let r = rr.clone();
        let conn = conn.clone();
        async move {
            r.focus(&conn, FocusReport { window_focused: p.window_focused, block_id: p.block_id.filter(|b| !b.is_empty()) });
            Ok(NotifyOk { ok: true })
        }
    });

    let rr = r.clone();
    engine.register_typed("notify.ack", move |p: NotifyAckParams, _ctx| {
        let r = rr.clone();
        async move { Ok(NotifyAckResult { has_window: r.ack(&p.id, p.clicked) }) }
    });

    let rr = r.clone();
    engine.register_typed("notify.test", move |_p: NotifyNoArgs, _ctx| {
        let r = rr.clone();
        async move {
            r.test();
            Ok(NotifyOk { ok: true })
        }
    });

    let rr = r;
    engine.register_typed("notify.takeactivation", move |_p: NotifyNoArgs, _ctx| {
        let r = rr.clone();
        async move { Ok(NotifyTakeActivationResult { block_id: r.take_pending_activation() }) }
    });
}
