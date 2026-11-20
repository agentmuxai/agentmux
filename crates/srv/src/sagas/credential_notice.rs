// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A container agent that can't get a credential, told to the person in the agent's own pane.
//!
//! A container agent has no credential of its own: its GitHub token and its AWS session are brokered by the host,
//! one use at a time. When the host refuses (GitHub isn't connected or the Cloud sign-in lapsed, the host's AWS login
//! lapsed), the agent reads the reason in its shell and nothing else shows it: the person looking at the pane sees an
//! agent that is stuck. This puts the same sentence the agent read on the pane as a line, with a link to Connectors
//! for GitHub, and takes it away when the credential is given again.
//!
//! It asks nothing and waits for nothing (no countdown, no buttons): it is the `agent:notice` line, kept until the
//! credential works or the person dismisses it. One line per cause per agent per hour, so a retry loop doesn't
//! stack lines. It is drawn from srv's own record: the agent never supplies the words.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use crate::server::AppState;
use agentmux_common::time::now_ms;

use super::agent_notice::NoticeView;

/// How long a line counts as "already shown" before a new refusal puts it up again.
const REPEAT_MS: i64 = 60 * 60 * 1000;
/// The most of the sentence the line carries (the tooltip carries the same).
const MAX_SENTENCE: usize = 240;

/// What the agent could not get.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cause {
    Github,
    Aws,
}

impl Cause {
    fn kind(self) -> &'static str {
        match self {
            Cause::Github => "github",
            Cause::Aws => "aws",
        }
    }
}

struct Shown {
    id: String,
    at_ms: i64,
}

static SHOWN: LazyLock<Mutex<HashMap<(String, Cause), Shown>>> = LazyLock::new(Default::default);

fn shown() -> std::sync::MutexGuard<'static, HashMap<(String, Cause), Shown>> {
    SHOWN.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a refusal does to the pane: nothing (the line is still up), or put a line up, taking down the old one first
/// when it is an hour old.
#[derive(Debug, PartialEq, Eq)]
struct Plan {
    take_down: Option<String>,
    put_up: String,
}

fn plan(map: &mut HashMap<(String, Cause), Shown>, slug: &str, cause: Cause, now: i64) -> Option<Plan> {
    let key = (slug.to_string(), cause);
    let old = map.get(&key);
    if old.is_some_and(|s| now - s.at_ms < REPEAT_MS) {
        return None;
    }
    let take_down = old.map(|s| s.id.clone());
    let put_up = uuid::Uuid::new_v4().to_string();
    map.insert(key, Shown { id: put_up.clone(), at_ms: now });
    Some(Plan { take_down, put_up })
}

fn sentence(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_SENTENCE {
        return one_line;
    }
    format!("{}…", one_line.chars().take(MAX_SENTENCE - 1).collect::<String>().trim_end())
}

fn view(id: &str, cause: Cause, block_id: &str, agent: &str, headline: &str, status: &'static str, now: i64) -> NoticeView {
    NoticeView {
        id: id.to_string(),
        kind: cause.kind(),
        block_id: block_id.to_string(),
        agent: agent.to_string(),
        headline: headline.to_string(),
        body: String::new(),
        quote: String::new(),
        always_label: String::new(),
        deadline_ms: 0,
        opened_ms: now,
        status,
        outcome: String::new(),
    }
}

fn publish(state: &AppState, v: &NoticeView) {
    state.broker.publish_event::<crate::backend::mps_events::AgentNotice>(v, vec![], 32);
}

/// The host refused `agent` (the agent with uid `uid`, slug `slug`) the credential for `cause`, with `said`: the very
/// sentence the agent was given. Shows it in the agent's pane unless it is already showing.
pub fn refused(state: &AppState, uid: &str, slug: &str, cause: Cause, said: &str) {
    // No pane open for the agent: nobody to tell, and a later refusal can still tell them.
    let Some(block_id) = state.reactive_handler.block_for_uid(uid) else { return };
    let now = now_ms();
    let Some(plan) = plan(&mut shown(), slug, cause, now) else { return };
    let headline = sentence(said);
    if let Some(old) = &plan.take_down {
        publish(state, &view(old, cause, &block_id, slug, &headline, "cleared", now));
    }
    publish(state, &view(&plan.put_up, cause, &block_id, slug, &headline, "line", now));
    tracing::info!(target: "identity", agent = %slug, kind = cause.kind(), "credential notice: shown in the agent's pane");
}

/// The host gave `agent` the credential for `cause`: takes its line down, if one is up.
pub fn worked(state: &AppState, uid: &str, slug: &str, cause: Cause) {
    let Some(up) = shown().remove(&(slug.to_string(), cause)) else { return };
    let block_id = state.reactive_handler.block_for_uid(uid).unwrap_or_default();
    publish(state, &view(&up.id, cause, &block_id, slug, "", "cleared", now_ms()));
}

#[cfg(test)]
#[path = "credential_notice_tests.rs"]
mod tests;
