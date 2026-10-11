// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Overlap notes, the runtime half
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.3, file half).
//!
//! The progress watcher hands over each sweep's new edits
//! ([`on_new_edits`]) and returns at once: the check runs on its own task.
//! There, every agent's work facts on this computer are gathered (this srv's
//! directly, other channels' through [`super::cross_channel`] with its
//! timeouts), each edit is matched ([`super::overlap`]), and a match becomes
//! one `[AgentMux]` system note to the agent that edited, rate-limited by
//! [`super::overlap::RateLimiter`]. Delivery is best-effort: a failure is
//! logged, never retried, and never reaches the watcher.
//!
//! Off with the setting `agent:overlapnotes: false`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use crate::backend::storage::store::Store;
use crate::backend::storage::work_claims::WorkClaim;
use crate::backend::wconfig::ConfigState;

use super::overlap::{self, Decision, RateLimiter};
use super::{collect_local, cross_channel, WorkFacts};

/// The setting that turns overlap notes off for this install.
pub const SETTING: &str = "agent:overlapnotes";

/// One edit the progress watcher saw happen live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEdit {
    pub agent_id: String,
    pub block_id: String,
    /// As the edit tool named it.
    pub path: String,
}

/// What the check needs from the running srv, set once it exists.
struct Context {
    store: Arc<Store>,
    /// The identity store, where every channel's work claims are.
    claims: Arc<Store>,
    config: Arc<ConfigState>,
    client: reqwest::Client,
    own_url: String,
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

fn limiter() -> &'static parking_lot::Mutex<RateLimiter> {
    static LIMITER: std::sync::LazyLock<parking_lot::Mutex<RateLimiter>> = std::sync::LazyLock::new(Default::default);
    &LIMITER
}

/// Turn the notes on for this srv. Until this is called (and in tests),
/// [`on_new_edits`] does nothing.
pub fn install(
    store: Arc<Store>,
    claims: Arc<Store>,
    config: Arc<ConfigState>,
    client: reqwest::Client,
    own_url: String,
) {
    let _ = CONTEXT.set(Context { store, claims, config, client, own_url });
}

/// Whether the notes are on (`agent:overlapnotes`, default on).
pub fn enabled(extra: &HashMap<String, serde_json::Value>) -> bool {
    extra.get("agent:overlapnotes").and_then(|v| v.as_bool()).unwrap_or(true)
}

/// The edits worth checking now: none when the notes are off, else those
/// whose (pane, file) wasn't checked in the last minute.
fn admit(
    edits: Vec<NewEdit>,
    extra: &HashMap<String, serde_json::Value>,
    limiter: &mut RateLimiter,
    now_ms: u64,
) -> Vec<NewEdit> {
    if !enabled(extra) {
        return Vec::new();
    }
    edits.into_iter().filter(|e| limiter.should_check(&e.block_id, &e.path, now_ms)).collect()
}

/// Run `check` on its own task and return at once, so the caller (the
/// watcher's loop) never waits on fetching facts or delivering a note.
fn dispatch<F, Fut>(edits: Vec<NewEdit>, check: F)
where
    F: FnOnce(Vec<NewEdit>) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    if edits.is_empty() {
        return;
    }
    match tokio::runtime::Handle::try_current() {
        Ok(rt) => {
            rt.spawn(check(edits));
        }
        Err(_) => tracing::debug!("overlap notes: no runtime; edits not checked"),
    }
}

/// The progress watcher's hand-off: this sweep's live edits. Returns at
/// once; see the module doc.
pub fn on_new_edits(edits: Vec<NewEdit>) {
    if edits.is_empty() {
        return;
    }
    let Some(ctx) = CONTEXT.get() else { return };
    let extra = ctx.config.get_settings().extra;
    let now = agentmux_common::time::now_ms_u64();
    let edits = admit(edits, &extra, &mut limiter().lock(), now);
    dispatch(edits, |edits| check_and_notify(ctx, edits));
}

/// Gather every agent's facts, match each edit, and deliver what the rate
/// limits allow.
async fn check_and_notify(ctx: &'static Context, edits: Vec<NewEdit>) {
    let handler = crate::backend::reactive::get_global_handler();
    let regs = handler.list_agents();
    let channel = cross_channel::own_channel();
    let store = ctx.store.clone();
    let regs_for_facts = regs.clone();
    let local = tokio::task::spawn_blocking(move || collect_local(&store, &regs_for_facts, &channel));
    let claims_store = ctx.claims.clone();
    let claims = tokio::task::spawn_blocking(move || {
        claims_store.work_claims_live(agentmux_common::time::now_ms()).unwrap_or_default()
    });
    let (local, claims, (cross, _)) = tokio::join!(local, claims, cross_channel::fetch(&ctx.client, &ctx.own_url));
    let mut all: Vec<WorkFacts> = local.unwrap_or_default();
    all.extend(cross);
    let claims = claims.unwrap_or_default();
    let now = agentmux_common::time::now_ms_u64();
    for edit in edits {
        let Some(text) = note_for(&edit, &all, &claims, &mut limiter().lock(), now) else { continue };
        // By UID when the pane has one, so a name two panes share still
        // reaches the one that edited.
        let target = regs
            .iter()
            .find(|r| r.block_id == edit.block_id)
            .and_then(|r| r.uid.clone())
            .unwrap_or_else(|| edit.agent_id.clone());
        if let Err(e) = handler.deliver_system_note(&target, &text) {
            tracing::info!(agent = %edit.agent_id, error = %e, "overlap note not delivered");
        }
    }
}

/// The note (if any) for one edit, given every agent's facts and the live
/// work claims.
fn note_for(
    edit: &NewEdit,
    all: &[WorkFacts],
    claims: &[WorkClaim],
    limiter: &mut RateLimiter,
    now_ms: u64,
) -> Option<String> {
    let me = all.iter().find(|f| f.block_id == edit.block_id);
    let file = overlap::locate(&edit.path, me, all)?;
    let found = overlap::overlaps(&file, &edit.agent_id, all, now_ms);
    let found = overlap::with_claims(found, &file, &edit.agent_id, claims);
    if found.is_empty() {
        return None;
    }
    match limiter.decide(&edit.agent_id, &file, &found, now_ms)? {
        Decision::Note(t) | Decision::Summary(t) => Some(t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 5 * 60 * 60 * 1000;

    fn edit(agent: &str, block: &str, path: &str) -> NewEdit {
        NewEdit { agent_id: agent.into(), block_id: block.into(), path: path.into() }
    }

    fn facts(agent: &str, block: &str, root: &str, dirty: &[&str]) -> WorkFacts {
        WorkFacts {
            agent: agent.into(),
            block_id: block.into(),
            repo: Some("agentmuxai/agentmux".into()),
            repo_root: Some(root.into()),
            branch: Some(format!("{}/x", agent.to_lowercase())),
            dirty_files: dirty.iter().map(|d| d.to_string()).collect(),
            ..Default::default()
        }
    }

    fn settings(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn the_setting_is_on_by_default_and_off_turns_every_check_off() {
        assert!(enabled(&settings(&[])));
        assert!(enabled(&settings(&[(SETTING, serde_json::json!(true))])));
        let off = settings(&[(SETTING, serde_json::json!(false))]);
        assert!(!enabled(&off));
        let mut rl = RateLimiter::default();
        assert!(admit(vec![edit("A", "b1", "/a/x.rs")], &off, &mut rl, NOW).is_empty());
        assert_eq!(admit(vec![edit("A", "b1", "/a/x.rs")], &settings(&[]), &mut rl, NOW).len(), 1);
    }

    #[test]
    fn an_edit_of_a_file_another_agent_has_uncommitted_gives_the_editor_one_note() {
        let all = vec![
            facts("AgentY", "b1", "C:/w/agenty/agentmux", &[]),
            facts("Agent4", "b2", "C:/w/agent4/agentmux", &["crates/srv/src/muxbus/presence.rs"]),
        ];
        let mut rl = RateLimiter::default();
        let e = edit("AgentY", "b1", r"C:\w\agenty\agentmux\crates\srv\src\muxbus\presence.rs");
        assert_eq!(
            note_for(&e, &all, &[], &mut rl, NOW).as_deref(),
            Some(
                "[AgentMux] Agent4 also has uncommitted changes in crates/srv/src/muxbus/presence.rs \
                 (repo agentmuxai/agentmux, branch agent4/x). Message them before changing it."
            )
        );
        assert_eq!(note_for(&e, &all, &[], &mut rl, NOW + 1000), None, "once per pair");

        // The other agent editing its own dirty file hears nothing about itself.
        let theirs = edit("Agent4", "b2", "C:/w/agent4/agentmux/crates/srv/src/muxbus/presence.rs");
        assert_eq!(note_for(&theirs, &all, &[], &mut rl, NOW), None);
        // A file outside every known repository is never matched.
        assert_eq!(note_for(&edit("AgentY", "b1", "D:/tmp/notes.md"), &all, &[], &mut rl, NOW), None);
    }

    /// The watcher's hand-off returns while the check is still running: a
    /// check that never finishes can't hold the sweep up.
    #[tokio::test]
    async fn the_check_runs_on_its_own_task() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<usize>();
        let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let t0 = std::time::Instant::now();
        dispatch(vec![edit("A", "b1", "/x")], move |edits| async move {
            let _ = started_tx.send(edits.len());
            let _ = release_rx.await; // never released while the caller waits
        });
        assert!(t0.elapsed() < std::time::Duration::from_millis(500), "dispatch waited on the check");
        assert_eq!(started_rx.await.unwrap(), 1, "the check still ran");
    }

    #[test]
    fn nothing_is_dispatched_without_edits_or_a_runtime() {
        let mut ran = false;
        dispatch(Vec::new(), |_| {
            ran = true;
            async {}
        });
        assert!(!ran);
        // Outside a runtime the check is dropped rather than panicking.
        dispatch(vec![edit("A", "b1", "/x")], |_| async {});
    }
}
