// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The task an entry happened under: each row of the Audit links to the Kanban task its agent held at that time, and
//! to that task's PR (SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md §3.4). The Kanban's own history knows
//! who held a task and when (`Store::work_held_at`); this puts the answer on the rows of a page.

use std::collections::HashMap;

use crate::backend::rpc_types::{AuditEntry, AuditTaskLink};
use crate::backend::storage::store::Store;

/// Give each entry (and each end of a jekt) the task its agent held when it happened, where `held` knows one.
pub fn attach(entries: &mut [AuditEntry], held: &dyn Fn(&str, u64) -> Option<AuditTaskLink>) {
    for e in entries {
        let at = e.ts_ms;
        if let Some(agent) = e.agent_id.as_deref() {
            e.links.task = held(agent, at);
        }
        for end in [e.from.as_mut(), e.to.as_mut()].into_iter().flatten() {
            end.links.task = held(&end.agent_id, at);
        }
    }
}

/// What the Kanban says about `agent` at `at_ms`: the task it held, as a link. A task is looked up once per page.
pub fn from_store(identity: &Store) -> impl Fn(&str, u64) -> Option<AuditTaskLink> + '_ {
    let cards: std::cell::RefCell<HashMap<String, Option<AuditTaskLink>>> = Default::default();
    move |agent, at_ms| {
        let id = identity.work_held_at(agent, i64::try_from(at_ms).ok()?).ok().flatten()?;
        cards
            .borrow_mut()
            .entry(id.clone())
            .or_insert_with(|| {
                let task = identity.work_board_get(&id).ok().flatten()?;
                let pr_url = Some(task.card.pr_url.trim().to_string()).filter(|u| !u.is_empty());
                Some(AuditTaskLink { num: task.card.num, task_ref: task.card.task_ref.clone(), pr_url })
            })
            .clone()
    }
}

#[cfg(test)]
#[path = "tasks_tests.rs"]
mod tests;
