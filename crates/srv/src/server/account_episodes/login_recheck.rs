// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A pane left signed out by a login that has since been signed in again clears itself (#47468).
//!
//! A pane whose last turn was refused on its login keeps that failure until a clean turn or a sign-in
//! that srv saw ([`super::after_sign_in`]) clears it, and a signed-out pane holds its messages, so no
//! turn can run to clear it. A person signing in from a pane's own CLI, or a credentials file that
//! came back, reached neither: the pane stayed "Not authenticated" for hours after the account read
//! signed in, and a restart did not help.
//!
//! On every tick, a signed-out pane whose account holds a token that is still good **and was issued
//! after the turn failed** (it expires later than the failure was recorded, so it is not the token
//! that was refused) has its failure cleared and is told, as after a sign-in. A token tried once for
//! a pane is not tried again for it: a login the provider refuses for another reason costs one turn,
//! not one a tick.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use crate::agents::failure::FailureClass;
use crate::backend::agent_state::failure_of_block;
use crate::backend::obj::Block;
use crate::backend::storage::identities::SecretRef;
use crate::identity::resolver::claude_token_expiry_ms;
use crate::server::AppState;

use super::machine::BlockedPane;
use super::resume::Failed;
use super::{canonical, AgentState};

/// For each pane, the token expiry it was last cleared for: the same token is not tried twice.
static TRIED: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(Default::default);

/// Whether a token expiring at `expiry_ms` is a new login for a turn that failed at `failed_at_ms`
/// (unix ms), at `now_ms`: still good now, and expiring after the failure. A failure stored without
/// its time is judged on the token being good now alone.
pub(super) fn is_new_login(expiry_ms: Option<i64>, failed_at_ms: Option<u64>, now_ms: i64) -> bool {
    let Some(expiry) = expiry_ms else { return false };
    if expiry <= now_ms {
        return false;
    }
    failed_at_ms.is_none_or(|at| expiry > i64::try_from(at).unwrap_or(i64::MAX))
}

/// Records that `expiry_ms` is tried for `block_id`; `false` when it already was.
pub(super) fn first_try(tried: &mut HashMap<String, i64>, block_id: &str, expiry_ms: i64) -> bool {
    if tried.get(block_id) == Some(&expiry_ms) {
        return false;
    }
    tried.insert(block_id.to_string(), expiry_ms);
    true
}

/// Clear the signed-out panes among `panes` whose login is new, and tell them. BLOCKING.
pub(super) fn run(state: &AppState, panes: &[BlockedPane]) {
    let signed_out: Vec<&BlockedPane> = panes.iter().filter(|p| matches!(p.state, AgentState::SignedOut) && canonical(&p.provider) == "claude").collect();
    if signed_out.is_empty() {
        TRIED.lock().unwrap_or_else(|e| e.into_inner()).clear();
        return;
    }
    let accounts = state.id_store.identity_list(None).unwrap_or_default();
    let now_ms = agentmux_common::time::now_ms();
    let mut tried = TRIED.lock().unwrap_or_else(|e| e.into_inner());
    tried.retain(|id, _| signed_out.iter().any(|p| &p.block_id == id));
    let mut cleared = Vec::new();
    for pane in signed_out {
        let Some(account) = accounts.iter().find(|a| a.id == pane.account_id) else { continue };
        let SecretRef::OAuthConfigDir { dir } = &account.secret_ref else { continue };
        let expiry = claude_token_expiry_ms(dir);
        let Some(block) = state.mstore.get::<Block>(&pane.block_id).ok().flatten() else { continue };
        let Some(failure) = failure_of_block(&block).filter(|f| f.code == FailureClass::Auth) else { continue };
        let Some(expiry) = expiry.filter(|_| is_new_login(expiry, failure.at_ms, now_ms)) else { continue };
        if !first_try(&mut tried, &pane.block_id, expiry) {
            continue;
        }
        crate::backend::blockcontroller::core::persist_last_failure(&block.oid, None, &Some(state.mstore.clone()), &Some(state.event_bus.clone()));
        cleared.push(Failed { block_id: block.oid.clone(), failed_at_ms: failure.at_ms });
    }
    drop(tried);
    if !cleared.is_empty() {
        tracing::info!(cleared = cleared.len(), "account episode: a login signed in again since a pane's turn failed cleared the pane's signed-out state");
        super::after_clearing();
        super::resume::start(cleared, now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_issued_after_the_failure_is_a_new_login() {
        // Failed at 1000 on a token that had expired; the file now holds one good until 9000 (now 2000).
        assert!(is_new_login(Some(9000), Some(1000), 2000));
    }

    #[test]
    fn a_token_that_is_expired_or_missing_is_not() {
        assert!(!is_new_login(Some(1500), Some(1000), 2000), "expired now");
        assert!(!is_new_login(None, Some(1000), 2000), "no token");
    }

    #[test]
    fn the_token_the_turn_was_refused_on_is_not_new() {
        // The turn failed at 5000; a token that expires at or before then was there when it failed.
        assert!(!is_new_login(Some(4000), Some(5000), 3000), "expiry not after the failure");
        assert!(!is_new_login(Some(5000), Some(5000), 3000), "expiry equal to the failure");
    }

    #[test]
    fn a_failure_without_its_time_is_judged_on_the_token_being_good() {
        assert!(is_new_login(Some(9000), None, 2000));
        assert!(!is_new_login(Some(1000), None, 2000));
    }

    #[test]
    fn a_token_is_tried_once_for_a_pane() {
        let mut tried = HashMap::new();
        assert!(first_try(&mut tried, "b1", 9000));
        assert!(!first_try(&mut tried, "b1", 9000), "the same token again");
        assert!(first_try(&mut tried, "b1", 12000), "a newer token");
        assert!(first_try(&mut tried, "b2", 9000), "another pane");
    }
}
