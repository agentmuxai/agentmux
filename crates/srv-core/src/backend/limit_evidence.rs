// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A clean turn on an account is evidence for the whole account.
//!
//! A pane whose run ended on the plan's usage limit stays `limited` until the provider's reset, a clean
//! turn of its own, a sign-in or an account move (`limit_wake`, `limit_probe`). When the plan is raised
//! or the limit lifted before the reset, other agents on the same account are already working, and
//! every pane still marked limited, and the account itself (its state is read from its panes' stored
//! failures, `account_blocks_at`), waited for the reset days away. So when a turn ends clean on an
//! account, every other pane of that account whose usage limit was stored before that turn began
//! has its failure cleared at once, the held-message replay wakes, and each of those agents is told
//! once to continue, staggered as the reset wake is.
//!
//! The scan is a map lookup unless some account has a stored limit: [`note_limited_blocks`] records
//! the accounts when a limit is stored and at start.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::agents::failure::{FailureClass, ROUTE_CREDITS_TITLE};
use crate::backend::agent_state::{failure_of_block, RUN_ACCOUNT_META_KEY};
use crate::backend::eventbus::EventBus;
use crate::backend::obj::Block;
use crate::backend::storage::store::Store;

/// A limit stored less than this before a turn ended clean is left alone: that turn may have been
/// served just before the limit was hit.
pub const GRACE_MS: u64 = 30_000;

/// The accounts with a pane whose stored failure is a usage limit.
static LIMITED_ACCOUNTS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// The account a pane's run launched on.
fn account_of(block: &Block) -> Option<String> {
    block.meta.get(RUN_ACCOUNT_META_KEY).and_then(|a| a.as_str()).filter(|a| !a.is_empty()).map(str::to_string)
}

/// Whether `block` holds a usage limit a clean turn on its account can clear: a plan's limit, not a
/// route out of credits (its own episode ends it, `route_credits`).
fn holds_plan_limit(block: &Block) -> bool {
    failure_of_block(block).is_some_and(|f| f.code == FailureClass::UsageLimit && f.title != ROUTE_CREDITS_TITLE)
}

/// Records the accounts of the panes in `blocks` that hold a usage limit.
pub fn note_limited_blocks(blocks: &[Block]) {
    let accounts: Vec<String> = blocks.iter().filter(|b| holds_plan_limit(b)).filter_map(account_of).collect();
    if accounts.is_empty() {
        return;
    }
    LIMITED_ACCOUNTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).extend(accounts);
}

fn is_noted(account: &str) -> bool {
    LIMITED_ACCOUNTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|set| set.contains(account))
}

fn forget(account: &str) {
    if let Some(set) = LIMITED_ACCOUNTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        set.remove(account);
    }
}

/// What a clean turn on `account` clears: the panes (other than the one that ran) whose usage limit
/// was stored at least [`GRACE_MS`] before `now_ms`, and whether any pane of the account still
/// holds one afterwards.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub clear: Vec<String>,
    pub still_limited: bool,
}

pub(crate) fn plan(blocks: &[Block], account: &str, clean_block: &str, now_ms: u64) -> Plan {
    let mut plan = Plan { clear: Vec::new(), still_limited: false };
    for block in blocks.iter().filter(|b| b.oid != clean_block && holds_plan_limit(b)) {
        if account_of(block).as_deref() != Some(account) {
            continue;
        }
        let stored_at = failure_of_block(block).and_then(|f| f.at_ms);
        // One stored before the field existed has no time: it is old.
        if stored_at.is_none_or(|at| at.saturating_add(GRACE_MS) <= now_ms) {
            plan.clear.push(block.oid.clone());
        } else {
            plan.still_limited = true;
        }
    }
    plan
}

/// A turn on `block_id` ended clean. When its account has other panes still marked limited, clears
/// them, wakes the replay and tells their agents to continue. Returns how many were cleared. A map
/// lookup when no account has a stored limit. BLOCKING when it has (store reads and writes).
pub fn clean_turn(block_id: &str, mstore: &Option<Arc<Store>>, event_bus: &Option<Arc<EventBus>>) -> usize {
    let Some(store) = mstore else { return 0 };
    if LIMITED_ACCOUNTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_none_or(HashSet::is_empty) {
        return 0;
    }
    let Some(account) = store.get::<Block>(block_id).ok().flatten().as_ref().and_then(account_of) else { return 0 };
    if !is_noted(&account) {
        return 0;
    }
    let blocks = store.get_all::<Block>().unwrap_or_default();
    let Plan { clear, still_limited } = plan(&blocks, &account, block_id, agentmux_common::time::now_ms_u64());
    for id in &clear {
        super::blockcontroller::core::persist_last_failure(id, None, mstore, event_bus);
    }
    if !still_limited {
        forget(&account);
    }
    if clear.is_empty() {
        return 0;
    }
    tracing::info!(cleared = clear.len(), "limit evidence: a clean turn on the account cleared the limit of its other panes");
    super::reactive::held_jekt_wake().notify_one();
    for (n, id) in clear.iter().enumerate() {
        super::limit_wake::nudge_after(id, n as u64 * super::limit_wake::NUDGE_STAGGER_MS);
    }
    clear.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(store: &Store, account: Option<&str>, failure: Option<serde_json::Value>) -> Block {
        let mut b = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
        if let Some(account) = account {
            b.meta.insert(RUN_ACCOUNT_META_KEY.to_string(), serde_json::json!(account));
        }
        if let Some(failure) = failure {
            b.meta.insert(crate::backend::blockcontroller::core::META_LAST_FAILURE.to_string(), failure);
        }
        store.insert(&mut b).unwrap();
        b
    }

    fn limit(at_ms: Option<u64>, resets_at_ms: Option<u64>) -> serde_json::Value {
        let mut f = serde_json::json!({"code": "usage_limit", "title": "Usage limit reached", "detail": "", "retryable": false});
        if let Some(at) = at_ms {
            f[crate::backend::blockcontroller::core::META_FAILURE_AT_MS] = serde_json::json!(at);
        }
        if let Some(r) = resets_at_ms {
            f["resetsAtMs"] = serde_json::json!(r);
        }
        f
    }

    const NOW: u64 = 10_000_000;

    #[test]
    fn a_clean_turn_clears_the_other_panes_of_its_account_that_were_limited_before_it() {
        let store = Store::open_in_memory().unwrap();
        let ran = pane(&store, Some("acct-a"), None);
        let old = pane(&store, Some("acct-a"), Some(limit(Some(NOW - 600_000), Some(NOW + 86_400_000))));
        let no_time = pane(&store, Some("acct-a"), Some(limit(None, None)));
        let other_account = pane(&store, Some("acct-b"), Some(limit(Some(NOW - 600_000), None)));
        let no_account = pane(&store, None, Some(limit(Some(NOW - 600_000), None)));
        let plan = plan(&[ran.clone(), old.clone(), no_time.clone(), other_account, no_account], "acct-a", &ran.oid, NOW);
        assert_eq!(plan, Plan { clear: vec![old.oid, no_time.oid], still_limited: false });
    }

    #[test]
    fn a_limit_stored_just_before_the_turn_ended_is_kept_and_keeps_the_account_noted() {
        let store = Store::open_in_memory().unwrap();
        let ran = pane(&store, Some("acct-a"), None);
        let fresh = pane(&store, Some("acct-a"), Some(limit(Some(NOW - GRACE_MS + 1), Some(NOW + 60_000))));
        assert_eq!(plan(&[ran.clone(), fresh], "acct-a", &ran.oid, NOW), Plan { clear: vec![], still_limited: true });
    }

    #[test]
    fn a_route_out_of_credits_and_other_failures_are_not_cleared_by_a_clean_turn() {
        let store = Store::open_in_memory().unwrap();
        let ran = pane(&store, Some("acct-a"), None);
        let credits = pane(
            &store,
            Some("acct-a"),
            Some(serde_json::json!({"code": "usage_limit", "title": ROUTE_CREDITS_TITLE, "detail": "", "retryable": false, "atMs": 1})),
        );
        let signed_out = pane(&store, Some("acct-a"), Some(serde_json::json!({"code": "auth", "title": "x", "detail": "", "retryable": false, "atMs": 1})));
        assert_eq!(plan(&[ran.clone(), credits, signed_out], "acct-a", &ran.oid, NOW), Plan { clear: vec![], still_limited: false });
    }

    #[test]
    fn a_clean_turn_clears_the_stored_failures_of_its_accounts_other_panes() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let account = format!("acct-{}", uuid::Uuid::new_v4());
        let ran = pane(&store, Some(&account), None);
        let limited = pane(&store, Some(&account), Some(limit(Some(1), Some(u64::MAX / 2))));
        let elsewhere = pane(&store, Some("acct-elsewhere"), Some(limit(Some(1), Some(u64::MAX / 2))));
        let (mstore, bus) = (Some(store.clone()), None);

        assert_eq!(clean_turn(&ran.oid, &mstore, &bus), 0, "nothing noted yet: nothing is scanned");
        note_limited_blocks(&store.get_all::<Block>().unwrap());
        assert_eq!(clean_turn(&ran.oid, &mstore, &bus), 1);

        let stored = |b: &Block| store.get::<Block>(&b.oid).unwrap().unwrap().meta.contains_key("agent:last_failure");
        assert!(!stored(&limited), "the account's other pane is no longer limited");
        assert!(stored(&elsewhere), "another account is not spoken for");
        assert_eq!(clean_turn(&ran.oid, &mstore, &bus), 0, "the account is forgotten once nothing on it is limited");
    }
}
