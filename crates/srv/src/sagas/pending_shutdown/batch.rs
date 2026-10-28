// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One window for several agents (`SPEC_COUNTDOWN_BUTTONS_AND_BATCH_AGENT_ACTIONS_2026_10_28.md` §4).
//! A `Stop agents` over five agents is one request: one deadline, one OS notification ("Closing 5
//! agents: A, B, C and 2 more in 30 s") with Keep running and Close now for all of them, and one
//! audit entry. Each agent's own pane keeps its banner, and its Keep running keeps that agent only.
//!
//! A block that already has an active request joins that one, as it always did; only the blocks no
//! request covers are put in the new one.

use serde::Serialize;

use super::*;

/// The notification group of a batch's toast: it is no one block's, so the request is its own.
pub(super) const GROUP_PREFIX: &str = "batch:";

pub(super) fn group(request_id: &str) -> String {
    format!("{GROUP_PREFIX}{request_id}")
}

/// An agent of a batch.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BatchBlock {
    pub block_id: String,
    /// Its display name, cleaned for the user.
    pub name: String,
}

/// What became of one agent of a batch.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockOutcome {
    pub block_id: String,
    pub name: String,
    /// `kept_by_user`, or how its close ended: `shut_down`, `superseded` (already gone) or `failed`.
    #[ts(type = r#""kept_by_user" | "shut_down" | "superseded" | "failed""#)]
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

/// The name the user knows the agent in `block_id` by.
fn block_name(state: &AppState, block_id: &str) -> String {
    let named = state
        .mstore
        .get::<crate::backend::obj::Block>(block_id)
        .ok()
        .flatten()
        .map(|b| crate::backend::obj::meta_get_string(&b.meta, "agentName", ""))
        .filter(|n| !n.trim().is_empty())
        .or_else(|| state.reactive_handler.get_agent_by_block(block_id).map(|a| a.agent_id))
        .unwrap_or_else(|| block_id.to_string());
    clean_by(&named)
}

/// What a toast calls the agents closing, within the notification's name limit: `lark`, or
/// `5 agents: A, B, C and 2 more` with as many names as fit.
pub(super) fn subject(names: &[&str]) -> String {
    const LIMIT: usize = 32;
    let n = names.len();
    if n == 1 {
        return names[0].to_string();
    }
    for shown in (1..=n).rev() {
        let list = names[..shown].join(", ");
        let text = if shown == n { format!("{n} agents: {list}") } else { format!("{n} agents: {list} and {} more", n - shown) };
        if text.chars().count() <= LIMIT {
            return text;
        }
    }
    format!("{n} agents")
}

/// Ask to close `blocks` (agent tabs) behind one window. Returns one view per block, in order: the
/// blocks put in the new request share its id (each view names its own block and agent), the others
/// are whatever request already covered them. With fewer than two blocks free it is the ordinary
/// request per block.
pub fn request_batch(state: &AppState, blocks: &[String], by: &str, reason: &str, detail: &str) -> Vec<PendingView> {
    let one = |b: &str| request(state, b, by, "FleetBulkStop", reason, Action::CloseTab { detail: detail.to_string() });
    let named: Vec<(String, String)> = blocks.iter().map(|b| (b.clone(), block_name(state, b))).collect();
    let shown_by = clean_by(&asker_name(state, by));
    let opened = {
        let mut map = entries();
        prune(&mut map, now_ms());
        let fresh: Vec<&(String, String)> = named
            .iter()
            .filter(|(b, _)| !map.values().any(|e| is_active(e.view.status) && e.covers.iter().any(|c| c == b)))
            .collect();
        if fresh.len() < 2 {
            None
        } else {
            let ids: Vec<String> = fresh.iter().map(|(b, _)| b.clone()).collect();
            let view = PendingView {
                request_id: uuid::Uuid::new_v4().to_string(),
                block_id: ids[0].clone(),
                by: shown_by,
                target: fresh.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(", "),
                via: "FleetBulkStop".to_string(),
                reason: clean_reason(reason),
                effect: "close_tab",
                deadline_ms: now_ms() + OVERRIDE_WINDOW.as_millis() as i64,
                status: "pending",
                error: None,
                blocks: Some(fresh.iter().map(|(b, n)| BatchBlock { block_id: b.clone(), name: n.clone() }).collect()),
                outcomes: None,
                joined: false,
            };
            map.insert(
                view.request_id.clone(),
                Entry {
                    view: view.clone(),
                    kept: Arc::new(tokio::sync::Notify::new()),
                    finished_at_ms: None,
                    action: Some(Action::CloseTabs { block_ids: ids.clone(), detail: detail.to_string() }),
                    requesters: vec![(by.to_string(), "FleetBulkStop".to_string(), reason.trim().to_string())],
                    covers: ids,
                    closing: false,
                },
            );
            Some(view)
        }
    };
    let Some(view) = opened else {
        return blocks.iter().map(|b| one(b)).collect();
    };
    for b in view.blocks.iter().flatten() {
        publish_pending(state, &b.block_id, &view);
    }
    raise_toast(state, &view);
    tracing::info!(request_id = %view.request_id, agents = view.blocks.as_ref().map_or(0, Vec::len), by, "shutdown pending for several agents: the user can keep any of them");
    spawn_window(state, view.clone(), OVERRIDE_WINDOW);
    let in_batch = |b: &str| view.blocks.iter().flatten().any(|x| x.block_id == b);
    blocks
        .iter()
        .map(|b| {
            if !in_batch(b) {
                return one(b);
            }
            let target = state.reactive_handler.get_agent_by_block(b).map(|a| a.agent_id).unwrap_or_else(|| b.clone());
            PendingView { block_id: b.clone(), target, ..view.clone() }
        })
        .collect()
}

/// The OS notification of a batch: what will happen to how many, the time left, and the two buttons.
pub(super) fn raise_toast(state: &AppState, view: &PendingView) {
    let Some(router) = notify_router(state) else { return };
    let names: Vec<&str> = view.blocks.iter().flatten().map(|b| b.name.as_str()).collect();
    if names.is_empty() {
        return;
    }
    let body = if names.len() == 1 {
        format!("{} asked to close it. It stays in My Agents, with its conversation.", view.by)
    } else {
        format!("{} asked to close them. They stay in My Agents, with their conversations.", view.by)
    };
    let info = crate::backend::notify::policy::ShutdownInfo {
        deadline_ms: view.deadline_ms,
        effect: view.effect,
        running: true,
        request_id: view.request_id.clone(),
        labels: None,
    };
    router.emit_group(&group(&view.request_id), &subject(&names), Some(body), info);
}

/// "Keep running" on one agent's banner of a batch: that agent leaves the request and the others and
/// the clock are untouched. When none is left the request is `kept_by_user`. `None` when the request
/// is not a batch or the block is not one of its agents (the whole-request keep applies).
pub(super) fn keep_member(state: &AppState, block_id: &str, request_id: &str) -> Option<&'static str> {
    let (view, left) = {
        let mut map = entries();
        let e = map.get_mut(request_id)?;
        if e.view.blocks.is_none() || !e.covers.iter().any(|b| b == block_id) {
            return None;
        }
        if e.view.status != "pending" || e.closing {
            return Some("too_late");
        }
        let name = e.view.blocks.iter().flatten().find(|b| b.block_id == block_id).map(|b| b.name.clone()).unwrap_or_default();
        e.covers.retain(|b| b != block_id);
        if let Some(Action::CloseTabs { block_ids, .. }) = e.action.as_mut() {
            block_ids.retain(|b| b != block_id);
        }
        if let Some(blocks) = e.view.blocks.as_mut() {
            blocks.retain(|b| b.block_id != block_id);
        }
        e.view.outcomes.get_or_insert_with(Vec::new).push(BlockOutcome { block_id: block_id.to_string(), name, outcome: "kept_by_user", error: None });
        if e.covers.is_empty() {
            e.view.status = "kept_by_user";
            e.finished_at_ms = Some(now_ms());
            e.kept.notify_one();
        }
        (e.view.clone(), e.covers.clone())
    };
    let data = PendingClearedEvent { block_id: block_id.to_string(), request_id: request_id.to_string(), outcome: "kept_by_user".to_string() };
    state.broker.publish_event::<crate::backend::mps_events::AgentShutdownPendingCleared>(&data, vec![format!("block:{block_id}")], 1);
    if left.is_empty() {
        if let Some(router) = notify_router(state) {
            router.resolve_nonblocking(&group(request_id), crate::backend::notify::policy::Family::Shutdown);
        }
        audit(state, request_id, &format!("kept_by_user ({})", summary(&view)), None);
    } else {
        // The banners and the toast name who is still closing.
        for b in &left {
            publish_pending(state, b, &view);
        }
        raise_toast(state, &view);
    }
    tracing::info!(block_id, request_id, "shutdown pending for several agents: one kept by the user");
    Some("kept_by_user")
}

/// The whole request kept (the toast's Keep running): every agent still in it is recorded as kept.
pub(super) fn mark_all_kept(e: &mut Entry) {
    let Some(blocks) = e.view.blocks.as_ref() else { return };
    let kept: Vec<BlockOutcome> = blocks
        .iter()
        .map(|b| BlockOutcome { block_id: b.block_id.clone(), name: b.name.clone(), outcome: "kept_by_user", error: None })
        .collect();
    e.view.outcomes.get_or_insert_with(Vec::new).extend(kept);
}

/// `A: kept_by_user, B: shut_down`: the outcomes as the audit note keeps them.
fn summary(view: &PendingView) -> String {
    view.outcomes.iter().flatten().map(|o| format!("{}: {}", o.name, o.outcome)).collect::<Vec<_>>().join(", ")
}

/// Close each agent still in the request, all at once as separate requests would have, and record how
/// each ended. The request's own status is `failed` if any close failed (with each error named),
/// else `shut_down` if any closed, else `superseded`.
pub(super) async fn run(state: &AppState, v: &PendingView, block_ids: Vec<String>, detail: String) -> (&'static str, Option<String>, String) {
    let name_of = |b: &str| v.blocks.iter().flatten().find(|x| x.block_id == b).map_or_else(|| b.to_string(), |x| x.name.clone());
    let closes = block_ids.iter().map(|b| {
        let detail = detail.clone();
        async move { run_action(state, b, Action::CloseTab { detail }).await }
    });
    let results = futures_util::future::join_all(closes).await;
    let ended: Vec<BlockOutcome> = block_ids
        .iter()
        .zip(results)
        .map(|(b, r)| {
            let (outcome, error) = match r {
                Ok(()) => ("shut_down", None),
                // Already gone: the user closed or stopped it meanwhile.
                Err(e) if e.contains("block not found") || e.starts_with("NOT_RUNNING") => ("superseded", None),
                Err(e) => ("failed", Some(e)),
            };
            BlockOutcome { block_id: b.clone(), name: name_of(b), outcome, error }
        })
        .collect();
    let failed: Vec<String> = ended.iter().filter(|o| o.outcome == "failed").map(|o| format!("{}: {}", o.name, o.error.as_deref().unwrap_or("close failed"))).collect();
    let status = if !failed.is_empty() {
        "failed"
    } else if ended.iter().any(|o| o.outcome == "shut_down") {
        "shut_down"
    } else {
        "superseded"
    };
    // Recorded before the status settles, so a waiter that sees the status sees every outcome.
    let all = {
        let mut map = entries();
        let Some(e) = map.get_mut(&v.request_id) else { return (status, (!failed.is_empty()).then(|| failed.join("; ")), status.to_string()) };
        e.view.outcomes.get_or_insert_with(Vec::new).extend(ended);
        e.view.clone()
    };
    (status, (!failed.is_empty()).then(|| failed.join("; ")), format!("{status} ({})", summary(&all)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("blk-batch-{i}-{}", uuid::Uuid::new_v4().simple())).collect()
    }

    #[test]
    fn the_toast_names_as_many_agents_as_fit() {
        assert_eq!(subject(&["lark"]), "lark");
        assert_eq!(subject(&["A", "B", "C", "D", "E"]), "5 agents: A, B, C, D, E");
        assert_eq!(subject(&["Alpha", "Bravo", "Charlie", "Delta", "Echo"]), "5 agents: Alpha and 4 more");
        assert_eq!(subject(&["A very long agent name indeed", "B"]), "2 agents");
    }

    /// Five targets, one window; the pane banners and the toast are that request's.
    #[tokio::test]
    async fn five_agents_are_one_request_and_each_view_names_its_own_block() {
        let state = test_state();
        let blocks = ids(5);
        let views = request_batch(&state, &blocks, "AgentY", "tidying", "asked by AgentY");
        assert_eq!(views.len(), 5);
        assert!(views.iter().all(|v| v.request_id == views[0].request_id && v.status == "pending"), "one window");
        assert_eq!(views.iter().map(|v| v.block_id.clone()).collect::<Vec<_>>(), blocks);
        assert_eq!(status(&views[0].request_id).unwrap().blocks.unwrap().len(), 5);
        for b in &blocks {
            assert_eq!(active_for(b).map(|v| v.request_id), Some(views[0].request_id.clone()));
        }
        assert_eq!(keep(&state, &group(&views[0].request_id), &views[0].request_id), "kept_by_user", "the toast keeps all");
        assert_eq!(status(&views[0].request_id).unwrap().outcomes.unwrap().len(), 5);
    }

    /// Keep running on one agent's banner keeps that agent only: four are left, the clock is the same.
    #[tokio::test]
    async fn keeping_one_leaves_the_others_and_the_clock() {
        let state = test_state();
        let blocks = ids(5);
        let views = request_batch(&state, &blocks, "AgentY", "", "");
        let id = views[0].request_id.clone();
        let deadline = views[0].deadline_ms;
        assert_eq!(keep(&state, &blocks[2], &id), "kept_by_user");
        let now = status(&id).unwrap();
        assert_eq!((now.status, now.deadline_ms), ("pending", deadline));
        assert_eq!(now.blocks.as_ref().unwrap().len(), 4);
        assert!(active_for(&blocks[2]).is_none(), "the kept agent is free again");
        assert_eq!(keep(&state, &blocks[2], &id), "too_late", "once");
        expire_now(&id);
        let done = settled(&id).await;
        let outcomes = done.outcomes.expect("every agent has an outcome");
        assert_eq!(outcomes.len(), 5);
        assert_eq!(outcomes.iter().filter(|o| o.outcome == "kept_by_user").map(|o| o.block_id.as_str()).collect::<Vec<_>>(), vec![blocks[2].as_str()]);
        assert!(outcomes.iter().filter(|o| o.outcome != "kept_by_user").count() == 4);
    }

    #[tokio::test]
    async fn keeping_the_last_agent_keeps_the_request() {
        let state = test_state();
        let blocks = ids(2);
        let views = request_batch(&state, &blocks, "AgentY", "", "");
        let id = views[0].request_id.clone();
        assert_eq!(keep(&state, &blocks[0], &id), "kept_by_user");
        assert_eq!(status(&id).unwrap().status, "pending");
        assert_eq!(keep(&state, &blocks[1], &id), "kept_by_user");
        assert_eq!(settled(&id).await.status, "kept_by_user");
    }

    /// Close now (the toast's second button) closes the rest at once; a second press is too late.
    #[tokio::test]
    async fn close_now_on_the_group_closes_every_agent_once() {
        let state = test_state();
        let blocks = ids(3);
        let views = request_batch(&state, &blocks, "AgentY", "", "");
        let id = views[0].request_id.clone();
        assert_eq!(close_now(&state, &group(&id), &id), "closed_now");
        let done = settled(&id).await;
        assert_eq!(done.outcomes.unwrap().len(), 3);
        assert_eq!(close_now(&state, &group(&id), &id), "too_late");
    }

    /// A block some request already covers joins that request; the others share a new one.
    #[tokio::test]
    async fn a_block_with_a_window_already_joins_it() {
        let state = test_state();
        let blocks = ids(3);
        let first = request(&state, &blocks[0], "Korp", "QuitSelf", "", Action::SelfQuit { detail: String::new() });
        let views = request_batch(&state, &blocks, "AgentY", "", "");
        assert_eq!(views[0].request_id, first.request_id);
        assert!(views[0].joined);
        assert_eq!(views[1].request_id, views[2].request_id);
        assert_ne!(views[1].request_id, first.request_id);
        assert_eq!(status(&views[1].request_id).unwrap().blocks.unwrap().len(), 2);
        keep(&state, &blocks[0], &first.request_id);
        keep(&state, &group(&views[1].request_id), &views[1].request_id);
    }

    /// With one block free there is nothing to batch: the ordinary request.
    #[tokio::test]
    async fn one_free_block_is_an_ordinary_request() {
        let state = test_state();
        let blocks = ids(2);
        let first = request(&state, &blocks[0], "Korp", "QuitSelf", "", Action::SelfQuit { detail: String::new() });
        let views = request_batch(&state, &blocks, "AgentY", "", "");
        assert!(views[1].blocks.is_none(), "not a batch");
        assert_ne!(views[1].request_id, first.request_id);
        keep(&state, &blocks[0], &first.request_id);
        keep(&state, &blocks[1], &views[1].request_id);
    }
}
