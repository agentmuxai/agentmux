// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The store the Kanban queue lives in, for code that has no `AppState` to read it from (a
//! controller building a continuation packet). It is the identity store, unless this is a build
//! with a work store of its own (`task dev`, or `AGENTMUX_ISOLATED_WORK=1`), where the queue's claims
//! are only in that one. Attached once at boot beside the row store (`identity_spawn::attach_row_store`).

use std::sync::{Arc, Mutex};

use crate::backend::storage::store::Store;

static WORK_STORE: Mutex<Option<Arc<Store>>> = Mutex::new(None);

/// Record `store` as the channel's work store. A later call replaces the earlier one.
pub fn attach(store: Arc<Store>) {
    *WORK_STORE.lock().unwrap_or_else(|e| e.into_inner()) = Some(store);
}

/// The work store [`attach`] recorded; `None` before boot does.
pub fn get() -> Option<Arc<Store>> {
    WORK_STORE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
