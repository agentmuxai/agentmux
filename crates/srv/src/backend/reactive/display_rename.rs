// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A running pane's registration when its agent is renamed
//! (`renameagentdefinitiontitle`): the display binding moves to the new name
//! at once, not at the agent's next turn, and the name it leaves is no longer
//! held as a current one. Its file registry entries go with it. Its relay
//! subscription is kept only while `live_names` holds it as a former name
//! (receive-only, for the former slugs' grace period) and is otherwise dropped
//! as orphaned by the lease tick. A message to the old name still reaches the
//! agent through the former-name resolver, and nothing speaks as it.
//!
//! A child of `handler` (through `former_slug`), for its private maps.

use super::super::{Handler, ReactiveHandler};

/// What a rename changed in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedBinding {
    pub block_id: String,
    /// The block's display binding now.
    pub display: String,
    /// The display binding it replaced, when no live block holds that name
    /// any more.
    pub retired: Option<String>,
    /// Every name the block is bound under now.
    pub names: Vec<String>,
}

impl Handler {
    /// Rebind the display binding of the block running `uid` from `old_title`
    /// to `title`, both as route names, keeping its stable name, UID, tab and
    /// registration nonce. `None` when no block runs it, or its display
    /// binding isn't `old_title` (a pane registered under another name keeps
    /// it), or the title doesn't change it.
    pub(crate) fn rebind_renamed(&mut self, uid: &str, old_title: &str, title: &str) -> Option<RenamedBinding> {
        let display = crate::backend::reactive::sanitize::registrable_id(title)?;
        let block_id = self.uid_to_block.get(uid)?.clone();
        let old = self.block_names.get(&block_id)?.display.clone();
        let was = crate::backend::reactive::sanitize::registrable_id(old_title)?;
        if !old.eq_ignore_ascii_case(&was) || old.eq_ignore_ascii_case(&display) {
            return None;
        }
        let (tab_id, nonce) =
            self.agent_info.get(&block_id).map(|r| (r.tab_id.clone(), r.registration_nonce)).unwrap_or_default();
        self.register_agent_full(&display, &block_id, tab_id.as_deref(), nonce, None, Some(uid), "registration.no_uid.rename")
            .ok()?;
        let names = self.names_for_block(&block_id);
        let still_held = names.iter().any(|n| n.eq_ignore_ascii_case(&old)) || self.name_to_blocks.contains_key(&old.to_lowercase());
        Some(RenamedBinding { block_id, display, retired: (!still_held).then_some(old), names })
    }
}

impl ReactiveHandler {
    /// See the inner [`Handler::rebind_renamed`].
    pub(crate) fn rebind_renamed(&self, uid: &str, old_title: &str, title: &str) -> Option<RenamedBinding> {
        self.inner.lock().unwrap().rebind_renamed(uid, old_title, title)
    }
}

/// After agent `uid` was renamed from `old_title` to `title`: rebind its
/// running pane ([`Handler::rebind_renamed`]) and move the name-keyed state a
/// turn's registration writes (`input.rs`) with it: the controller's own id,
/// the same-channel and cross-channel registry files, and the relay
/// subscription of the new names. `local_url` is this instance's.
pub(crate) fn renamed(uid: &str, old_title: &str, title: &str, local_url: &str) -> Option<RenamedBinding> {
    let r = crate::backend::reactive::get_global_handler().rebind_renamed(uid, old_title, title)?;
    if let Some(ctrl) = crate::backend::blockcontroller::get_controller(&r.block_id) {
        ctrl.set_agent_id(Some(r.display.clone()));
    }
    let data_dir = crate::backend::base::get_mux_data_dir();
    if let Some(old) = &r.retired {
        crate::backend::reactive::registry::remove(&data_dir, old);
        crate::backend::reactive::registry::remove_shared_from_env(old);
    }
    crate::backend::reactive::registry::write(&data_dir, &r.display, local_url, &r.block_id);
    crate::backend::reactive::registry::write_shared_from_env(&r.display, local_url, &r.block_id);
    crate::muxbus::cloud_subscriber::live_names::names_changed();
    if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
        r.names.iter().for_each(|n| sub.add_agent(n));
    }
    tracing::info!(uid, block_id = %r.block_id, display = %r.display, retired = ?r.retired, "reactive: renamed agent's pane rebound to its new name");
    Some(r)
}
