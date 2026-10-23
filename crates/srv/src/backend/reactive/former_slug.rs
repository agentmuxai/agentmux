// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Delivery to a slug an agent was renamed from
//! (`docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` Phase 1): a name
//! nothing is registered under is asked of the store, and a former slug
//! reaches its agent's registration by UID. Tried only after every live name
//! and current slug has missed, so it never takes a name from its holder, and
//! it adds nothing to a delivery that resolves as before. Only delivery asks
//! for it (`resolve_target`, [`Handler::get_delivery_target`],
//! [`Handler::lookup_delivery_target`]); `get_agent` and `lookup_by_name`,
//! which authenticate and act as an agent, never answer a former slug.
//!
//! And a former slug never speaks: [`refuse_former_slug_sender`] holds that
//! rule for every delivery, whichever path brought it. The handler applies it
//! to each message it delivers (`deliver_audited`), so a path that hands the
//! handler a request directly can't skip it; the server applies it too while
//! verifying, before a request is forwarded.
//!
//! The resolver answers for a display name an agent was renamed from too, by
//! its route (`Store::former_name_owner`), under the same rules: delivery
//! only, never a sender. When a rename retires the display name a running
//! pane is registered under, [`renamed`] moves the registration to the new one.
//!
//! A child of `handler`, for its private maps.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{Handler, LookupOutcome, ReactiveHandler};
use crate::backend::reactive::types::AgentRegistration;
use crate::backend::reactive::types::InjectionRequest;

/// The UID a name is a former slug or display name of
/// (`Store::former_name_owner`), `None` when it is not one. Runs under the
/// handler's lock, on a miss only.
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
    tracing::info!(former_name = name, uid, "reactive: delivered to a former name (logged once an hour per name)");
}

/// Count a request whose sender claims a former slug as unverified
/// (`sig_verified = false`): forced sensitive and stopped for the human,
/// whatever it was signed with and whatever tier it came by, since a former
/// slug can't sign. `is_live` says whether a live registration holds the
/// name (a live name is its holder's, never an alias); `owner_of` gives the
/// UID it is a former slug of. `true` when it refused the claim.
pub(crate) fn refuse_former_slug_sender(
    req: &mut InjectionRequest,
    is_live: impl FnOnce(&str) -> bool,
    owner_of: impl FnOnce(&str) -> Option<String>,
) -> bool {
    let Some(claimed) = req.source_agent.clone() else {
        return false;
    };
    let Some(uid) = former_slug_claim(&claimed, is_live, owner_of) else {
        return false;
    };
    let claimed = claimed.trim();
    tracing::warn!(
        claimed = %claimed,
        uid = %uid,
        tier = req.delivery_tier.as_deref().unwrap_or("host"),
        "jekt claims a name its agent was renamed from; a former name can't sign, counting it unverified"
    );
    req.sig_verified = Some(false);
    true
}

/// The UID `claimed` is a former slug of, when a signer names itself by one:
/// not a live name (`is_live`, lowercased), and a former slug (`owner_of`).
/// The rule [`refuse_former_slug_sender`] applies to a jekt, for any other
/// "verify a signature, then act as its signer" path (UI automation).
pub(crate) fn former_slug_claim(
    claimed: &str,
    is_live: impl FnOnce(&str) -> bool,
    owner_of: impl FnOnce(&str) -> Option<String>,
) -> Option<String> {
    let claimed = claimed.trim().to_lowercase();
    if claimed.is_empty() || is_live(&claimed) {
        return None;
    }
    owner_of(&claimed)
}

impl Handler {
    /// [`Handler::get_agent`], plus a former slug's agent: for a delivery
    /// target only, never a name that authenticates or acts.
    pub fn get_delivery_target(&self, name: &str) -> Option<&AgentRegistration> {
        match self.read_resolve(name, true) {
            Ok(Some(block)) => self.agent_info.get(&block),
            _ => None,
        }
    }

    /// [`Handler::lookup_by_name`], plus a former slug's agent: for a
    /// delivery target (a transcript, files, the resolve endpoint).
    pub fn lookup_delivery_target(&self, name: &str) -> LookupOutcome {
        self.lookup_resolved(name, true)
    }

    /// [`refuse_former_slug_sender`] against this registry and the store.
    pub(super) fn refuse_former_slug_sender(&self, req: &mut InjectionRequest) -> bool {
        let Some(resolver) = self.former_slug_resolver.as_ref() else {
            return false;
        };
        refuse_former_slug_sender(req, |name| self.name_to_blocks.contains_key(name), |name| resolver(name))
    }

    /// `resolve_target` for a name no block is bound under: its former slug's
    /// agent, logged ([`note_delivery`]), else not found.
    pub(super) fn former_slug_target(&self, key: &str, target: &str) -> super::TargetResolution {
        match self.former_slug_block(key) {
            Some((block, uid)) => {
                note_delivery(target, &uid);
                super::TargetResolution::FormerSlug { block, uid }
            }
            None => super::TargetResolution::NotFound,
        }
    }

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
    /// See the inner [`Handler::get_delivery_target`].
    pub fn get_delivery_target(&self, name: &str) -> Option<AgentRegistration> {
        self.inner.lock().unwrap().get_delivery_target(name).cloned()
    }

    /// See the inner [`Handler::lookup_delivery_target`].
    pub fn lookup_delivery_target(&self, name: &str) -> LookupOutcome {
        self.inner.lock().unwrap().lookup_delivery_target(name)
    }

    /// See the inner [`Handler::set_former_slug_resolver`].
    pub fn set_former_slug_resolver(&self, resolver: FormerSlugResolver) {
        self.inner.lock().unwrap().set_former_slug_resolver(resolver);
    }
}

#[path = "display_rename.rs"]
mod display_rename;
pub(crate) use display_rename::renamed;

#[cfg(test)]
#[path = "former_slug_tests.rs"]
mod tests;
