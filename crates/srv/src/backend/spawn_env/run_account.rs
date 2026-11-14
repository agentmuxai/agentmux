// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The account a pane's run launched on, noted on its block at spawn.

use serde_json::json;

use crate::backend::agent_state::RUN_ACCOUNT_META_KEY;
use crate::backend::obj::{self, Block, MetaMapType};
use crate::backend::storage::store::Store;

/// Note on `block_id` the account its spawn resolves: its agent's link for
/// its provider, as the identity injection resolves it. Best-effort; a pane
/// without it falls back to the agent's current link.
pub fn note_run_account(mstore: &Store, identity_store: &Store, block_id: &str) {
    use crate::backend::providers::resolve_provider_alias;
    let Ok(Some(block)) = mstore.get::<Block>(block_id) else {
        return;
    };
    let agent = obj::meta_get_string(&block.meta, "agentId", "");
    let Some(def) = mstore.agent_def_list().ok().and_then(|d| d.into_iter().find(|d| !agent.is_empty() && d.id == agent)) else {
        return;
    };
    let provider = resolve_provider_alias(&def.provider).to_string();
    let account = crate::identity::resolver::effective_identity_links(mstore, identity_store, &def)
        .unwrap_or_default()
        .into_iter()
        .find(|l| resolve_provider_alias(&l.provider) == provider)
        .map(|l| json!(l.account_id))
        .unwrap_or(serde_json::Value::Null);
    if block.meta.get(RUN_ACCOUNT_META_KEY).unwrap_or(&serde_json::Value::Null) == &account {
        return;
    }
    let mut meta = MetaMapType::new();
    meta.insert(RUN_ACCOUNT_META_KEY.to_string(), account);
    if let Err(e) = crate::backend::wcore::object_meta::update_object_meta(mstore, &format!("block:{block_id}"), &meta) {
        tracing::warn!(block_id, error = %e, "agent.spawn: couldn't note the account a pane runs on");
    }
}
