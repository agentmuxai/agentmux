// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the account episodes (`server/account_episodes`) publish and the layers below the
//! server read: the `accountavailability` event with the agents and tasks it names, and the
//! question "does an episode speak for this pane's failure", which the notification router asks
//! and the server answers once the episodes run.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// The event an episode's change is published as.
pub const EVENT_ACCOUNT_AVAILABILITY: &str = "accountavailability";

/// One agent of an episode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountEpisodeAgent {
    pub block_id: String,
    /// Its definition id: the UID a Kanban task's holder carries.
    pub agent_id: String,
    pub agent_name: String,
}

/// A Kanban task an episode's agent holds (claimed, or blocked on a question).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountEpisodeTask {
    pub id: String,
    #[ts(type = "number")]
    pub number: i64,
    /// How the task is named: `#12` on the default board, else `work#12`.
    pub task_ref: String,
    pub title: String,
}

/// `accountavailability` (spec §6.1): an episode opened, changed or closed
/// (`ok`). A signal for the windows, which then read
/// `identity.account.availability`: it carries no account label and no move
/// candidates, since any connection can subscribe to an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountAvailabilityEvent {
    pub account_id: String,
    pub provider: String,
    #[ts(type = r#""limited" | "signed_out" | "ok""#)]
    pub state: String,
    #[ts(type = "number | null")]
    pub resets_at_ms: Option<u64>,
    pub agents: Vec<AccountEpisodeAgent>,
    pub tasks: Vec<AccountEpisodeTask>,
}

static COVERS_BLOCK: OnceLock<fn(&str) -> bool> = OnceLock::new();

/// Sets the answer to [`covers_block`]; the server calls it once at startup.
pub fn set_covers_block(f: fn(&str) -> bool) {
    let _ = COVERS_BLOCK.set(f);
}

/// Whether an account episode speaks for this pane's failure, so its own `AgentCrashed` or
/// "finished" toast is left out (spec §6.2). `false` until the episodes run.
pub fn covers_block(block_id: &str) -> bool {
    COVERS_BLOCK.get().is_some_and(|f| f(block_id))
}
