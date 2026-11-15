// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The transcript record for a message srv delivered to an agent, with where it came from
//! (docs/specs/SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md §8, P3).
//!
//! The CLIs srv writes this record for run as one process per turn
//! ([`UserRecordSink`](super::subprocess::UserRecordSink)); an ACP agent and the Codex App Server keep
//! a session open and never put the user's words in their output, so without it their transcript holds
//! no trace of what was sent and no way to say who sent it.

use std::sync::Arc;

use crate::backend::message_origin::RecordOrigin;
use crate::backend::mps;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

use super::subprocess::{injected_user_record, user_record};

/// The agent's transcript, written by `write`: where its records go and how to find its zone.
#[derive(Clone)]
pub(super) struct PromptRecords {
    pub broker: Option<Arc<mps::Broker>>,
    pub filestore: Option<Arc<FileStore>>,
    pub mstore: Option<Arc<Store>>,
}

impl PromptRecords {
    /// Store `message` as a user record of `block_id`'s transcript and the agent's global zone, signed
    /// with `record`'s origin. `shown_by_pane`: the pane typed it and already shows it, so the record is
    /// published for the stream's sake only; otherwise it is published for the pane to show.
    ///
    /// Call it only once the message was handed to the agent: a transcript must not claim a turn that
    /// never happened.
    pub(super) fn write(&self, block_id: &str, message: &str, shown_by_pane: bool, record: Option<&RecordOrigin>) {
        let Some(store) = self.mstore.as_ref() else {
            return;
        };
        let Ok(block) = store.must_get::<crate::backend::obj::Block>(block_id) else {
            return;
        };
        let global_zone = crate::backend::agent_session::agent_zone_for_block_meta(&block.meta);
        let value = if shown_by_pane { user_record(message) } else { injected_user_record(message) };
        let line = format!("{value}\n");
        let persist = if shown_by_pane {
            crate::backend::transcript::append::persist_user_line
        } else {
            crate::backend::transcript::append::persist_injected_user_line
        };
        persist(self.broker.as_deref(), block_id, line.as_bytes(), self.filestore.as_ref(), global_zone.as_deref(), record);
    }
}
