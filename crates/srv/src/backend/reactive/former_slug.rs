// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Delivery to a slug an agent was renamed from
//! (`docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` Phase 1): a name
//! nothing is registered under is asked of the store, and a former slug
//! reaches its agent's registration by UID. Tried only after every live name
//! and current slug has missed, so it never takes a name from its holder, and
//! it adds nothing to a delivery that resolves as before.
//!
//! A child of `handler`, for its private maps.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{Handler, ReactiveHandler};

/// The UID a name is a former slug of (`Store::former_slug_owner`), `None`
/// when it is not one. Runs under the handler's lock, on a miss only.
pub type FormerSlugResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// How often one name's delivery through its former slug is logged.
const LOG_EVERY: Duration = Duration::from_secs(60 * 60);

/// Log a delivery that resolved through a former slug, once per name per
/// [`LOG_EVERY`]: an operator can see which old ids are still in use (the
/// contract phase waits for them to fall silent) without a line per message.
pub(super) fn note_delivery(name: &str, uid: &str) {
    static LAST: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
    let key = name.to_lowercase();
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let last = last.get_or_insert_with(HashMap::new);
    if last.get(&key).is_some_and(|at| at.elapsed() < LOG_EVERY) {
        return;
    }
    last.insert(key, Instant::now());
    tracing::info!(former_slug = name, uid, "reactive: delivered to a former slug (logged once an hour per name)");
}

impl Handler {
    /// See [`FormerSlugResolver`].
    pub fn set_former_slug_resolver(&mut self, resolver: FormerSlugResolver) {
        self.former_slug_resolver = Some(resolver);
    }

    /// The block and UID `name` reaches as a former slug: its agent is
    /// registered here by UID. Read-only; the caller logs a delivery.
    pub(super) fn former_slug_block(&self, name: &str) -> Option<(String, String)> {
        let uid = self.former_slug_resolver.as_ref()?(name)?;
        let block = self.uid_to_block.get(&uid)?.clone();
        Some((block, uid))
    }
}

impl ReactiveHandler {
    /// See the inner [`Handler::set_former_slug_resolver`].
    pub fn set_former_slug_resolver(&self, resolver: FormerSlugResolver) {
        self.inner.lock().unwrap().set_former_slug_resolver(resolver);
    }
}

#[cfg(test)]
#[path = "former_slug_tests.rs"]
mod tests;
