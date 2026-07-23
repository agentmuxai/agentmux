// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Session-id capture and hydration state machine for [`SubprocessController`].
//!
//! Two independent sources can set `inner.session_id`:
//!   - `hydrate_session_id_from_config`: best-effort seed from the picker
//!     "My Agents" reattach path (config-supplied, may be stale).
//!   - `record_captured_session_id[_inner]`: authoritative capture from the
//!     CLI's own stdout init event — always wins, and always overwrites a
//!     prior hydrated value.

use std::sync::Mutex;

use super::{SubprocessController, SubprocessControllerInner};

impl SubprocessController {
    /// Record an authoritative session id captured from the CLI's
    /// stdout init/`thread.started` event. The CLI is the source of
    /// truth for which session is live, so this ALWAYS overwrites
    /// any prior value of `inner.session_id` — including values
    /// previously hydrated from config on a picker reattach (which
    /// may be stale by the time the CLI speaks).
    ///
    /// Free-function form (taking `&Arc<Mutex<…Inner>>` instead of
    /// `&self`) so the spawn_turn stdout-reader tokio task can call
    /// it without holding an `Arc<Self>` reference. The
    /// `&SubprocessController` method below just delegates.
    ///
    /// Returns `true` when the value changed (caller should
    /// broadcast the meta update + persist to block meta). Returns
    /// `false` when the new id matches the current one — common
    /// when the CLI emits the same `session_id` on every NDJSON
    /// frame within a single turn.
    ///
    /// `pub(super)` (not `pub(crate)`): only called from the
    /// `subprocess` module tree (`mod.rs`/`host_spawn`/`container_spawn`/
    /// `tests`) — matching the module-private `SubprocessControllerInner`
    /// parameter's own reachability avoids a `private_interfaces` lint.
    pub(super) fn record_captured_session_id_inner(
        inner: &Mutex<SubprocessControllerInner>,
        sid: &str,
    ) -> bool {
        if sid.is_empty() {
            return false;
        }
        let mut guard = inner.lock().unwrap();
        let differs = guard.session_id.as_deref() != Some(sid);
        if differs {
            guard.session_id = Some(sid.to_string());
        }
        differs
    }

    /// `&self` convenience wrapper around
    /// `record_captured_session_id_inner` — used by tests that
    /// already hold a `SubprocessController`.
    #[cfg(test)]
    pub(crate) fn record_captured_session_id(&self, sid: &str) -> bool {
        Self::record_captured_session_id_inner(&self.inner, sid)
    }

    /// Hydrate `inner.session_id` from a config-supplied id when the
    /// controller hasn't seen a value yet.
    ///
    /// Picker reattach path: a fresh `SubprocessController` is
    /// registered for the new block, so its `inner.session_id` is
    /// `None`. The frontend persisted the prior block's session id
    /// into `agent:sessionid` meta, the websocket / app_api caller
    /// read it into `SubprocessSpawnConfig::session_id`, and this
    /// method copies it to inner so the spawn_turn args-builder
    /// appends `--resume <sid>` on the FIRST turn.
    ///
    /// **Hydration is best-effort, not authoritative.** If
    /// `inner.session_id` is already `Some` we no-op (don't overwrite
    /// a value already in place — could be a captured-from-stdout
    /// id from an earlier turn, or a prior hydration on the same
    /// reattach). Critically, the **CLI's stdout-emitted session id
    /// is authoritative** and overwrites any prior value at capture
    /// time (see the stdout-reader block in `spawn_turn`). So if the
    /// hydrated value is stale, the FIRST turn passes the stale id
    /// via `--resume` (likely accepted as a no-op or rejected with a
    /// "no such session" error from the CLI), the CLI then emits its
    /// own session id in the init event, and `inner.session_id` is
    /// overwritten with that authoritative value for subsequent
    /// turns. Without the capture overwrite, a stale hydrated id
    /// would be re-used forever — that was the bug codex flagged on
    /// PR #1018 first cut.
    ///
    /// Empty `&str` is treated as "no value" so the caller can use
    /// it unconditionally without filtering.
    pub(crate) fn hydrate_session_id_from_config(&self, config_sid: Option<&str>) {
        let Some(sid) = config_sid.filter(|s| !s.is_empty()) else {
            return;
        };
        let mut inner = self.inner.lock().unwrap();
        if inner.session_id.is_some() {
            return;
        }
        tracing::info!(
            block_id = %self.block_id,
            session_id = %sid,
            "hydrated session_id from config (picker reattach)"
        );
        inner.session_id = Some(sid.to_string());
    }
}
