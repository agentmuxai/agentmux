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
//! The PR half (§3.3): when the watcher sees an agent open a PR
//! ([`on_new_prs`]), the files its branch changes are compared with other
//! agents' uncommitted changes, recent edits and claims, and with the files
//! committed on their own branches of the same repository on this computer
//! (usually their open PRs). Local facts only: srv has no GitHub identity.
//! Open PRs from other computers are `WhoIsWorkingOn`'s, through the agent's
//! own `gh-agent`. One note per (pane, branch) per 2 hours.
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

/// An agent the progress watcher saw open a PR (`gh-agent pr create`,
/// `gh pr create`), live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPr {
    pub agent_id: String,
    pub block_id: String,
}

/// One PR note per (pane, branch) in this long.
const PR_NOTE_EVERY_MS: u64 = 2 * 60 * 60 * 1000;
/// Other clones whose branch files are read, at most, per PR.
const MAX_BRANCH_CLONES: usize = 8;

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

/// The progress watcher's hand-off: PRs being opened this sweep. Returns at
/// once; the check runs on its own task.
pub fn on_new_prs(prs: Vec<NewPr>) {
    if prs.is_empty() {
        return;
    }
    let Some(ctx) = CONTEXT.get() else { return };
    if !enabled(&ctx.config.get_settings().extra) {
        return;
    }
    match tokio::runtime::Handle::try_current() {
        Ok(rt) => {
            for pr in prs {
                rt.spawn(check_pr(ctx, pr));
            }
        }
        Err(_) => tracing::debug!("overlap notes: no runtime; PRs not checked"),
    }
}

fn pr_notes_sent() -> &'static parking_lot::Mutex<HashMap<(String, String), u64>> {
    static SENT: OnceLock<parking_lot::Mutex<HashMap<(String, String), u64>>> = OnceLock::new();
    SENT.get_or_init(Default::default)
}

/// Whether (pane, branch) may get a PR note now, and if so, record it.
fn admit_pr_note(sent: &mut HashMap<(String, String), u64>, block_id: &str, branch: &str, now_ms: u64) -> bool {
    sent.retain(|_, at| now_ms.saturating_sub(*at) < PR_NOTE_EVERY_MS);
    let key = (block_id.to_string(), branch.to_string());
    if sent.contains_key(&key) {
        return false;
    }
    sent.insert(key, now_ms);
    true
}

async fn check_pr(ctx: &'static Context, pr: NewPr) {
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

    let Some(me) = all.iter().find(|f| f.block_id == pr.block_id).cloned() else { return };
    let (Some(root), Some(repo_key), Some(branch)) = (me.repo_root.clone(), me.repo_key(), me.branch.clone()) else {
        return;
    };
    let files = super::git::branch_files(&root).await;
    if files.is_empty() {
        return;
    }
    // Other clones of the same repository, on a branch of their own.
    let mut theirs: Vec<(String, String, String)> = Vec::new();
    for f in &all {
        if theirs.len() >= MAX_BRANCH_CLONES {
            break;
        }
        let (Some(their_root), Some(their_branch)) = (f.repo_root.as_deref(), f.branch.as_deref()) else { continue };
        let same_repo = f.repo_key().as_deref() == Some(repo_key.as_str());
        if !same_repo || f.agent.eq_ignore_ascii_case(&me.agent) || their_root == root || their_branch == branch {
            continue;
        }
        if theirs.iter().any(|(_, r, _)| r == their_root) {
            continue;
        }
        theirs.push((f.agent.clone(), their_root.to_string(), their_branch.to_string()));
    }
    let mut branch_files = Vec::new();
    for (agent, their_root, their_branch) in theirs {
        let changed = super::git::branch_files(&their_root).await;
        branch_files.push((agent, their_branch, changed));
    }
    let repo = me.repo.clone().unwrap_or_else(|| root.clone());
    let now = agentmux_common::time::now_ms_u64();
    let found = pr_overlaps(&me.agent, &repo_key, &repo, &files, &all, &claims, &branch_files, now);
    if found.is_empty() || !admit_pr_note(&mut pr_notes_sent().lock(), &pr.block_id, &branch, now) {
        return;
    }
    let text = overlap::pr_note_text(&branch, &found);
    let target = regs
        .iter()
        .find(|r| r.block_id == pr.block_id)
        .and_then(|r| r.uid.clone())
        .unwrap_or_else(|| pr.agent_id.clone());
    if let Err(e) = handler.deliver_system_note(&target, &text) {
        tracing::info!(agent = %pr.agent_id, error = %e, "PR overlap note not delivered");
    }
}

/// Every other agent working on the PR's `files`: how, per agent, strongest
/// first (uncommitted, edited, claimed, on its branch). `branch_files`: other
/// agents' (name, branch, files their branch changes).
#[allow(clippy::too_many_arguments)]
fn pr_overlaps(
    me_agent: &str,
    repo_key: &str,
    repo: &str,
    files: &[String],
    all: &[WorkFacts],
    claims: &[WorkClaim],
    branch_files: &[(String, String, Vec<String>)],
    now_ms: u64,
) -> Vec<overlap::PrOverlap> {
    #[derive(Default)]
    struct Ways {
        name: String,
        dirty: Vec<String>,
        edited: Vec<String>,
        claims: Vec<String>,
        branch: Option<(String, Vec<String>)>,
    }
    let mut by_agent: Vec<Ways> = Vec::new();
    fn ways<'a>(by_agent: &'a mut Vec<Ways>, name: &str) -> &'a mut Ways {
        if let Some(i) = by_agent.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
            return &mut by_agent[i];
        }
        by_agent.push(Ways { name: name.to_string(), ..Default::default() });
        by_agent.last_mut().expect("just pushed")
    }
    for rel in files {
        let file = overlap::EditedFile { repo_key: repo_key.to_string(), repo: repo.to_string(), rel: rel.clone() };
        let found = overlap::with_claims(overlap::overlaps(&file, me_agent, all, now_ms), &file, me_agent, claims);
        for o in found {
            let w = ways(&mut by_agent, &o.agent);
            match o.kind {
                overlap::OverlapKind::Dirty => w.dirty.push(rel.clone()),
                overlap::OverlapKind::Edited => w.edited.push(rel.clone()),
                overlap::OverlapKind::Claimed => {}
            }
            if let Some(c) = o.claim {
                if !w.claims.contains(&c.what) {
                    w.claims.push(c.what);
                }
            }
        }
    }
    for (agent, their_branch, changed) in branch_files {
        let shared: Vec<String> = files.iter().filter(|f| changed.contains(f)).cloned().collect();
        if !shared.is_empty() {
            ways(&mut by_agent, agent).branch = Some((their_branch.clone(), shared));
        }
    }
    by_agent
        .into_iter()
        .filter_map(|w| {
            let mut doing = Vec::new();
            if !w.dirty.is_empty() {
                doing.push(format!("has uncommitted changes in {}", overlap::files_phrase(&w.dirty)));
            }
            if !w.edited.is_empty() {
                doing.push(format!("recently edited {}", overlap::files_phrase(&w.edited)));
            }
            for c in &w.claims {
                doing.push(format!("claimed {c}"));
            }
            if let Some((b, shared)) = &w.branch {
                doing.push(format!("changes {} on its branch {b}", overlap::files_phrase(shared)));
            }
            (!doing.is_empty()).then_some(overlap::PrOverlap { agent: w.name, doing })
        })
        .collect()
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

    #[test]
    fn a_pr_names_each_agent_working_on_its_files_and_how() {
        let mut agent4 = facts("Agent4", "b4", "/w4/agentmux", &["crates/a.rs"]);
        agent4.recent_files = vec![super::super::FactFile {
            path: "/w4/agentmux/crates/b.rs".into(),
            rel: Some("crates/b.rs".into()),
            tool: "Edit".into(),
            ts_ms: NOW - 60_000,
        }];
        let all = vec![facts("AgentY", "by", "/wy/agentmux", &[]), agent4, facts("Agent5", "b5", "/w5/agentmux", &[])];
        let files: Vec<String> = ["crates/a.rs", "crates/b.rs", "crates/c.rs"].iter().map(|s| s.to_string()).collect();
        let branch_files = vec![("Agent5".to_string(), "agent5/x".to_string(), vec!["crates/c.rs".to_string(), "other.rs".to_string()])];
        let got = pr_overlaps("AgentY", "agentmuxai/agentmux", "agentmuxai/agentmux", &files, &all, &[], &branch_files, NOW);
        assert_eq!(got, vec![
            overlap::PrOverlap {
                agent: "Agent4".into(),
                doing: vec!["has uncommitted changes in crates/a.rs".into(), "recently edited crates/b.rs".into()],
            },
            overlap::PrOverlap { agent: "Agent5".into(), doing: vec!["changes crates/c.rs on its branch agent5/x".into()] },
        ]);
        let text = overlap::pr_note_text("agenty/x", &got);
        assert!(text.starts_with("[AgentMux] The PR you are opening from agenty/x changes files other agents are working on: Agent4 has uncommitted changes in crates/a.rs and recently edited crates/b.rs; Agent5 changes crates/c.rs on its branch agent5/x."), "{text}");
        assert!(!text.contains('\n'));
    }

    #[test]
    fn a_pr_nobody_else_touches_gets_no_note() {
        let all = vec![facts("AgentY", "by", "/wy/agentmux", &[]), facts("Agent4", "b4", "/w4/agentmux", &["elsewhere.rs"])];
        let files = vec!["crates/a.rs".to_string()];
        assert!(pr_overlaps("AgentY", "agentmuxai/agentmux", "agentmuxai/agentmux", &files, &all, &[], &[], NOW).is_empty());
    }

    #[test]
    fn one_pr_note_per_pane_and_branch_every_two_hours() {
        let mut sent = HashMap::new();
        assert!(admit_pr_note(&mut sent, "b", "agenty/x", NOW));
        assert!(!admit_pr_note(&mut sent, "b", "agenty/x", NOW + 60_000));
        assert!(admit_pr_note(&mut sent, "b", "agenty/y", NOW + 60_000), "another branch");
        assert!(admit_pr_note(&mut sent, "b", "agenty/x", NOW + PR_NOTE_EVERY_MS + 1));
    }
}
