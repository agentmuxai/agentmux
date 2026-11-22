// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The "what the agent is on" line of an OS toast: never a model's paraphrase.
//!
//! The line is the agent's own status line (`Presence status`) when it is fresh, else the Kanban
//! task it holds, else nothing. A toast with only the agent's name and what happened is correct; a
//! line that might be old is left out, never shown. The old source, the ambient summarizer's stored
//! title (`term:ambient_summary`), is not read here: nothing checked its age, so a title written
//! about earlier work rode on today's toast.
//!
//! A status line is fresh when it was set during or after the turn the toast is about, or is not
//! stale by the Swarm's own rule (`LIVE_PHRASE_STALE_MS` in `frontend/app/store/swarm-line.ts`).

use crate::backend::obj::{self, Block};
use crate::backend::storage::store::Store;
use crate::backend::work_facts::live_phrase::{self, Declared};

use super::router::sanitize_summary;

/// A status line older than this is stale, as in the Swarm (30 minutes).
pub(crate) const STALE_MS: i64 = 30 * 60_000;

/// A Kanban task the agent holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Held {
    pub id: String,
    /// How the task is written (`#47659`, or `board#12`).
    pub reference: String,
    pub title: String,
    /// When the claim last changed the task (epoch ms): the best order the queue keeps.
    pub taken_ms: i64,
}

/// Whether the agent's own line may be shown on a toast raised at `now_ms`. A line the srv wrote
/// from a task claim is not the agent's own: the held task speaks for it.
fn own_line_is_fresh(line: &Declared, turn_started_ms: Option<i64>, now_ms: i64) -> bool {
    if line.src != live_phrase::SRC_AGENT || line.at_ms == 0 {
        return false;
    }
    let at = i64::try_from(line.at_ms).unwrap_or(i64::MAX);
    turn_started_ms.is_some_and(|started| at >= started) || now_ms.saturating_sub(at) <= STALE_MS
}

/// The held task that speaks for the agent: the one its line names (what it claimed last, while the
/// line is still the claim's), else the one the queue changed last.
fn task_line_of<'a>(held: &'a [Held], line: Option<&Declared>) -> Option<&'a Held> {
    let named = line.and_then(|l| l.task.as_deref()).and_then(|id| held.iter().find(|h| h.id == id));
    named.or_else(|| held.iter().max_by_key(|h| h.taken_ms))
}

/// The line for a toast, sanitized like any text an agent wrote (one line, no control characters,
/// capped, dropped when it trips the credential list), or `None`.
pub(crate) fn pick(line: Option<&Declared>, turn_started_ms: Option<i64>, held: &[Held], now_ms: i64) -> Option<String> {
    let own = line.filter(|l| own_line_is_fresh(l, turn_started_ms, now_ms)).and_then(|l| sanitize_summary(&l.text));
    own.or_else(|| {
        let task = task_line_of(held, line)?;
        sanitize_summary(&format!("{} {}", task.reference, task.title))
    })
}

/// The tasks `agent_id` (with `uid`, when known) holds, from the work store. Blocking reads.
fn held_in(work: &Store, agent_id: &str, uid: &str) -> Vec<Held> {
    crate::backend::continuity_facts::held_tasks(work, agent_id, uid)
        .into_iter()
        .map(|(reference, item)| Held { id: item.id, reference, title: item.title, taken_ms: item.updated_at })
        .collect()
}

/// The tasks the agent in `block` holds. A pane with no agent holds none. Blocking reads.
fn held_by(block: &Block) -> Vec<Held> {
    let Some(agent_id) = obj::block_meta_agent_id(&block.meta).map(str::trim).filter(|s| !s.is_empty()) else { return Vec::new() };
    let Some(work) = crate::backend::work_store_handle::get() else { return Vec::new() };
    let uid = crate::backend::reactive::get_global_handler()
        .list_agents()
        .into_iter()
        .find(|r| r.block_id == block.oid)
        .and_then(|r| r.uid)
        .unwrap_or_default();
    held_in(&work, agent_id, &uid)
}

/// The line for a toast about the agent in `block`. Blocking reads: only from `spawn_blocking`.
pub(crate) fn of_block(block: &Block, turn_started_ms: Option<i64>, now_ms: i64) -> Option<String> {
    let line = live_phrase::declared_of(&block.meta);
    pick(line.as_ref(), turn_started_ms, &held_by(block), now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10_000_000_000;
    const MIN: i64 = 60_000;

    fn agent_line(text: &str, at_ms: i64) -> Declared {
        Declared { text: text.into(), at_ms: at_ms as u64, state: None, src: live_phrase::SRC_AGENT.into(), task: None }
    }

    fn task(id: &str, reference: &str, title: &str, taken_ms: i64) -> Held {
        Held { id: id.into(), reference: reference.into(), title: title.into(), taken_ms }
    }

    #[test]
    fn a_fresh_status_line_is_the_line() {
        let line = agent_line("Fixing the toast line", NOW - 5 * MIN);
        let held = [task("t1", "#1", "Something else", NOW - 60 * MIN)];
        assert_eq!(pick(Some(&line), None, &held, NOW).as_deref(), Some("Fixing the toast line"));
    }

    #[test]
    fn a_status_line_older_than_the_swarms_limit_is_left_out() {
        let old = agent_line("Fixing the toast line", NOW - STALE_MS - MIN);
        assert_eq!(pick(Some(&old), None, &[], NOW), None, "no task: no line at all");
    }

    #[test]
    fn the_limit_itself_is_still_fresh() {
        let edge = agent_line("Fixing the toast line", NOW - STALE_MS);
        assert!(pick(Some(&edge), None, &[], NOW).is_some());
    }

    #[test]
    fn a_status_line_older_than_the_turn_and_than_the_limit_is_stale() {
        let started = NOW - 2 * MIN;
        let old = agent_line("Reviewing PR 6190", NOW - 3 * 60 * MIN);
        assert_eq!(pick(Some(&old), Some(started), &[], NOW), None);
    }

    #[test]
    fn a_status_line_set_during_a_long_turn_is_fresh() {
        // The turn began three hours ago and the line was set in it, two hours ago.
        let started = NOW - 180 * MIN;
        let line = agent_line("Waiting for the build", NOW - 120 * MIN);
        assert_eq!(pick(Some(&line), Some(started), &[], NOW).as_deref(), Some("Waiting for the build"));
    }

    #[test]
    fn a_line_of_no_known_age_is_stale() {
        assert_eq!(pick(Some(&agent_line("Reviewing PR 6190", 0)), None, &[], NOW), None);
    }

    #[test]
    fn without_a_status_line_the_held_task_speaks() {
        let held = [task("t1", "#48395", "OS toasts: stop showing the stale summary", NOW - 3 * MIN)];
        assert_eq!(pick(None, None, &held, NOW).as_deref(), Some("#48395 OS toasts: stop showing the stale summary"));
    }

    #[test]
    fn a_stale_status_line_gives_way_to_the_held_task() {
        let old = agent_line("Reviewing PR 6190", NOW - 5 * 60 * MIN);
        let held = [task("t1", "#7", "Build: a thing", NOW - 9 * 60 * MIN)];
        assert_eq!(pick(Some(&old), None, &held, NOW).as_deref(), Some("#7 Build: a thing"));
    }

    #[test]
    fn a_line_srv_wrote_from_a_claim_is_not_trusted_without_the_task() {
        let claim = Declared {
            text: "#7 Build: a thing".into(),
            at_ms: (NOW - MIN) as u64,
            state: None,
            src: live_phrase::SRC_TASK.into(),
            task: Some("t1".into()),
        };
        assert_eq!(pick(Some(&claim), None, &[], NOW), None, "the task ended: nothing is held");
        let held = [task("t1", "#7", "Build: a thing", NOW - MIN)];
        assert_eq!(pick(Some(&claim), None, &held, NOW).as_deref(), Some("#7 Build: a thing"));
    }

    #[test]
    fn of_several_tasks_the_one_the_line_names_wins_else_the_latest() {
        let held = [task("a", "#1", "Old one", NOW - 90 * MIN), task("b", "#2", "Newer one", NOW - 10 * MIN)];
        assert_eq!(pick(None, None, &held, NOW).as_deref(), Some("#2 Newer one"));
        let names_a = Declared { text: "#1 Old one".into(), at_ms: 1, state: None, src: live_phrase::SRC_TASK.into(), task: Some("a".into()) };
        assert_eq!(pick(Some(&names_a), None, &held, NOW).as_deref(), Some("#1 Old one"));
    }

    #[test]
    fn neither_a_line_nor_a_task_is_no_line() {
        assert_eq!(pick(None, None, &[], NOW), None);
    }

    fn work_item(id: &str, title: &str) -> crate::backend::storage::work_queue::WorkItem {
        use crate::backend::storage::work_queue::WorkItem;
        WorkItem {
            id: id.into(),
            title: title.into(),
            payload: "do it".into(),
            kind: String::new(),
            target_agent: String::new(),
            target_group: String::new(),
            priority: 0,
            state: "open".into(),
            claimed_by: String::new(),
            target_agent_uid: String::new(),
            claimed_by_uid: String::new(),
            claim_expires: None,
            attempts: 0,
            max_attempts: 3,
            created_by: "someone".into(),
            created_by_uid: String::new(),
            created_at: 1,
            updated_at: 1,
            not_before: None,
            result: String::new(),
        }
    }

    #[test]
    fn the_tasks_an_agent_holds_come_from_the_work_store_latest_claim_first() {
        use crate::backend::storage::work_queue::ClaimFilter;
        let dir = tempfile::tempdir().unwrap();
        let work = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
        let lark = ClaimFilter { agent_id: "lark".into(), agent_uid: "uid-lark".into(), ..ClaimFilter::default() };
        for (id, title, at) in [("a", "First task", 1_000), ("b", "Second task", 2_000)] {
            work.work_queue_enqueue(&work_item(id, title)).unwrap();
            let got = work.work_queue_claim(&lark, at, 120_000).unwrap().expect("claimed");
            assert_eq!(got.id, id);
        }
        // Another agent's task, and one nobody holds, are not lark's.
        work.work_queue_enqueue(&work_item("c", "Not claimed")).unwrap();
        let held = held_in(&work, "lark", "uid-lark");
        assert_eq!(held.len(), 2, "{held:?}");
        let shown = pick(None, None, &held, 3_000).unwrap();
        assert!(shown.ends_with("Second task"), "{shown}");
        assert!(held_in(&work, "other", "uid-other").is_empty());
    }

    #[test]
    fn another_identity_with_the_same_name_does_not_lend_its_task() {
        use crate::backend::storage::work_queue::ClaimFilter;
        let dir = tempfile::tempdir().unwrap();
        let work = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
        let theirs = ClaimFilter { agent_id: "lark".into(), agent_uid: "uid-other".into(), ..ClaimFilter::default() };
        work.work_queue_enqueue(&work_item("x", "Someone else's task")).unwrap();
        work.work_queue_claim(&theirs, 1_000, 120_000).unwrap().expect("claimed");
        // Same name, a different UID: the UID decides, so this lark holds nothing.
        assert!(held_in(&work, "lark", "uid-lark").is_empty());
        assert_eq!(held_in(&work, "lark", "uid-other").len(), 1);
        // With no UID known on our side, the name still matches.
        assert_eq!(held_in(&work, "lark", "").len(), 1);
    }

    #[test]
    fn the_ambient_title_is_never_the_line() {
        let mut block = Block { oid: "b1".into(), ..Default::default() };
        block.meta.insert(crate::ambient::title::META_TITLE.to_string(), serde_json::json!("Set up CI for the docs site"));
        block.meta.insert("agentId".to_string(), serde_json::json!("lark"));
        assert_eq!(of_block(&block, None, NOW), None, "a stored title, and nothing the agent said");
        // The agent's own fresh line shows, and the stale title still does not.
        let own = live_phrase::set_meta("Fixing the toast line", None, live_phrase::SRC_AGENT, NOW - MIN);
        for (k, v) in own.as_object().unwrap() {
            block.meta.insert(k.clone(), v.clone());
        }
        assert_eq!(of_block(&block, None, NOW).as_deref(), Some("Fixing the toast line"));
    }

    #[test]
    fn what_is_shown_is_one_bounded_line_without_a_credential_pattern() {
        let messy = agent_line("two\nlines\u{7}\u{202E}here", NOW - MIN);
        assert_eq!(pick(Some(&messy), None, &[], NOW).as_deref(), Some("two lines here"));
        let long = task("t", "#9", &"word ".repeat(60), NOW);
        let shown = pick(None, None, &[long], NOW).unwrap();
        assert!(shown.chars().count() <= 91 && shown.ends_with('…'), "{shown}");
        // A status line that trips the credential list is dropped; the task then speaks.
        let secret = agent_line("Rotating the api_key for staging", NOW - MIN);
        let held = [task("t", "#9", "Rotate staging", NOW)];
        assert_eq!(pick(Some(&secret), None, &held, NOW).as_deref(), Some("#9 Rotate staging"));
        assert_eq!(pick(Some(&secret), None, &[], NOW), None);
    }
}
