// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The account links a definition resolves through, for callers outside the
//! spawn gate (a new fork copies its source's from here).

use crate::backend::storage::store::{AgentDefinition, AgentIdentityLink, Store};
use crate::backend::storage::StoreError;

use super::inject::template_parent_id_if_seeded;

/// The account links a definition's spawn resolves through: its own direct
/// links, or, only when it has none, its seeded template parent's. The same
/// rule `resolve_bindings_for_instance` applies at the spawn gate, without
/// its "no links" diagnostic. `forkagentdefinition` copies a new fork's links
/// from this, so the fork starts on the account its source runs on.
pub fn effective_identity_links(
    mstore: &Store,
    identity_store: &Store,
    def: &AgentDefinition,
) -> Result<Vec<AgentIdentityLink>, StoreError> {
    let direct = identity_store.agent_identity_list_for_agent(&def.id)?;
    if !direct.is_empty() {
        return Ok(direct);
    }
    let parent = template_parent_id_if_seeded(mstore, &def.parent_id);
    if parent.is_empty() {
        return Ok(Vec::new());
    }
    identity_store.agent_identity_list_for_agent(&parent)
}
