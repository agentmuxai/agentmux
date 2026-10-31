// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `tasks.enqueue` and `tasks.list` (spec §4.3, phase P3): file a task on this machine's board for
//! one of its agents, and read the board. The logic is `work_remote`; these only give it the
//! transport's verified caller.

use serde_json::Value;

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::server::work_remote::{self, Failure};

fn refusal(f: Failure) -> RemoteRefusal {
    RemoteRefusal::new(f.code, f.message)
}

pub(super) async fn enqueue(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    work_remote::tasks_enqueue(state, &caller.actor(), params).await.map_err(refusal)
}

pub(super) async fn list(state: &AppState, _caller: &RemoteCaller, params: Value) -> RemoteResult {
    work_remote::tasks_list(state, params).map_err(refusal)
}
