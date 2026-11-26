// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Several apps asked for in one `Desktop` call. Each app is still its own
//! sensitive request, with its own card, countdown, veto and grant, and nothing
//! about one decides another. What the group changes is only what the agent is
//! told: the notes for the requests of one call are held, and when every one
//! has ended the agent gets a single note naming each app as allowed, declined,
//! vetoed or not decided.

use std::sync::Mutex;

use crate::backend::sensitive_requests::{self as requests, Request};

/// The most apps one call asks for.
pub(crate) const MAX_MEMBERS: usize = 8;
/// The most groups waiting at once, across all agents.
const MAX_GROUPS: usize = 32;
/// A group that has not finished by then is flushed: the agent is told what is known, and its
/// requests go back to sending their own notes. Longer than any review plus countdown.
pub(crate) const GROUP_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// How a request of a group ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Settled {
    /// The grant was made.
    Allowed,
    /// The grant could not be made, and why.
    Failed(String),
    /// The super agent said no.
    Declined,
    /// The person stopped it in the countdown.
    Vetoed,
    /// Withdrawn, or ended with no answer.
    Ended,
}

#[derive(Debug, Clone)]
struct Member {
    request_id: String,
    app: String,
    outcome: Option<Settled>,
}

#[derive(Debug, Clone)]
struct Group {
    /// The asking agent, its pane and the call's own id, so one agent's call can't be joined by another.
    key: String,
    block_id: String,
    members: Vec<Member>,
    /// The call has filed all its requests. Until then a request that ends early does not finish the group.
    sealed: bool,
}

static GROUPS: Mutex<Vec<Group>> = Mutex::new(Vec::new());

/// The key of one call's group.
pub(crate) fn key(uid: &str, block_id: &str, call: &str) -> String {
    format!("{uid}|{block_id}|{call}")
}

/// What `add` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Added {
    /// Not added (full, sealed, or too many groups): the request sends its own notes.
    No,
    /// Joined a group already there.
    Joined,
    /// Made the group: the caller starts its deadline.
    Created,
}

impl Added {
    /// The request is in a group.
    pub(crate) fn grouped(self) -> bool {
        self != Added::No
    }
}

/// Add the request filed for `app` to the group of this call, making the group if it is the first.
pub(crate) fn add(key: &str, block_id: &str, request_id: &str, app: &str) -> Added {
    let mut groups = GROUPS.lock().unwrap_or_else(|e| e.into_inner());
    let member = Member { request_id: request_id.to_string(), app: app.to_string(), outcome: None };
    if let Some(g) = groups.iter_mut().find(|g| g.key == key) {
        if g.sealed || g.members.len() >= MAX_MEMBERS || g.members.iter().any(|m| m.request_id == request_id) {
            return Added::No;
        }
        g.members.push(member);
        return Added::Joined;
    }
    if groups.len() >= MAX_GROUPS {
        return Added::No;
    }
    groups.push(Group { key: key.to_string(), block_id: block_id.to_string(), members: vec![member], sealed: false });
    Added::Created
}

/// Whether `request_id` is part of a group, so its own notes are held.
pub(crate) fn is_member(request_id: &str) -> bool {
    GROUPS.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|g| g.members.iter().any(|m| m.request_id == request_id))
}

/// The pane and the note to send when the group is finished, and the group is gone.
type Done = (String, String);

/// Every member ended: it has an outcome, or its request is no longer open.
fn finished(g: &Group, is_open: &dyn Fn(&str) -> bool) -> bool {
    g.sealed && g.members.iter().all(|m| m.outcome.is_some() || !is_open(&m.request_id))
}

fn take_if_finished(groups: &mut Vec<Group>, at: usize, is_open: &dyn Fn(&str) -> bool) -> Option<Done> {
    if !finished(&groups[at], is_open) {
        return None;
    }
    let g = groups.remove(at);
    Some((g.block_id.clone(), note(&g, is_open)))
}

/// End the group now whatever is left, as its deadline does: the note says what is known, and an
/// app still waiting says so and will send its own note when it is decided. Nothing happens if the
/// group already finished.
pub(crate) fn flush(key: &str, is_open: &dyn Fn(&str) -> bool) -> Option<Done> {
    let mut groups = GROUPS.lock().unwrap_or_else(|e| e.into_inner());
    let at = groups.iter().position(|g| g.key == key)?;
    let g = groups.remove(at);
    Some((g.block_id.clone(), note(&g, is_open)))
}

/// Record how `request_id` ended. Returns the note to send when that finished its group.
pub(crate) fn settle(request_id: &str, outcome: Settled, is_open: &dyn Fn(&str) -> bool) -> Option<Done> {
    let mut groups = GROUPS.lock().unwrap_or_else(|e| e.into_inner());
    let at = groups.iter().position(|g| g.members.iter().any(|m| m.request_id == request_id))?;
    if let Some(m) = groups[at].members.iter_mut().find(|m| m.request_id == request_id) {
        m.outcome.get_or_insert(outcome);
    }
    take_if_finished(&mut groups, at, is_open)
}

/// The call has filed every request: from now on the group may finish. Returns the note when every one
/// has already ended.
pub(crate) fn seal(key: &str, is_open: &dyn Fn(&str) -> bool) -> Option<Done> {
    let mut groups = GROUPS.lock().unwrap_or_else(|e| e.into_inner());
    let at = groups.iter().position(|g| g.key == key)?;
    groups[at].sealed = true;
    take_if_finished(&mut groups, at, is_open)
}

/// `app` and what became of it.
fn line(m: &Member, is_open: &dyn Fn(&str) -> bool) -> String {
    let said = match &m.outcome {
        Some(Settled::Allowed) => "allowed, you may use it now".to_string(),
        Some(Settled::Failed(why)) => format!("could not be allowed: {}", crate::backend::reactive::sanitize::quote_in_system_note(why)),
        Some(Settled::Declined) => "declined".to_string(),
        Some(Settled::Vetoed) => "vetoed by the person".to_string(),
        None if is_open(&m.request_id) => "still waiting for an answer; its own note will follow".to_string(),
        Some(Settled::Ended) | None => "not decided (withdrawn or timed out)".to_string(),
    };
    format!("{}: {said}", m.app)
}

fn note(g: &Group, is_open: &dyn Fn(&str) -> bool) -> String {
    let lines: Vec<String> = g.members.iter().map(|m| line(m, is_open)).collect();
    format!("[AgentMux] Your request to control {} apps has been answered: {}. Each answer is for that app alone.", g.members.len(), lines.join("; "))
}

/// Whether a request is still open in the registry.
pub(crate) fn request_is_open(request_id: &str) -> bool {
    requests::registry().get(request_id).is_some_and(|r| r.status.is_open())
}

/// The `Request` of a note being sent is held when it belongs to a group.
pub(crate) fn holds(request: &Request) -> bool {
    is_member(&request.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request is open until the test says it ended.
    fn open_all(_: &str) -> bool {
        true
    }

    fn fresh(name: &str) -> String {
        key("agent-uid", "block-1", name)
    }

    #[test]
    fn one_note_names_each_app_allowed_declined_or_vetoed() {
        let k = fresh("mixed");
        assert!(add(&k, "block-1", "mixed-a", "cantabile").grouped());
        assert!(add(&k, "block-1", "mixed-b", "voicemeeter").grouped());
        assert!(add(&k, "block-1", "mixed-c", "notepad").grouped());
        assert_eq!(seal(&k, &open_all), None);
        // The first two ending does not finish it, and sends nothing.
        assert_eq!(settle("mixed-a", Settled::Allowed, &open_all), None);
        assert_eq!(settle("mixed-b", Settled::Declined, &open_all), None);
        let (block, said) = settle("mixed-c", Settled::Vetoed, &open_all).expect("the last one ends the group");
        assert_eq!(block, "block-1");
        assert!(said.contains("cantabile: allowed"), "{said}");
        assert!(said.contains("voicemeeter: declined"), "{said}");
        assert!(said.contains("notepad: vetoed by the person"), "{said}");
        assert!(said.contains("Each answer is for that app alone"), "{said}");
        // The group is gone: nothing more is held.
        assert!(!is_member("mixed-a"));
    }

    #[test]
    fn an_app_is_allowed_only_by_its_own_outcome() {
        let k = fresh("alone");
        assert!(add(&k, "block-1", "alone-a", "cantabile").grouped());
        assert!(add(&k, "block-1", "alone-b", "voicemeeter").grouped());
        seal(&k, &open_all);
        // One allowed and one that timed out: the second is never reported as allowed.
        settle("alone-a", Settled::Allowed, &open_all);
        let (_, said) = settle("alone-b", Settled::Ended, &open_all).unwrap();
        assert!(said.contains("cantabile: allowed"));
        assert!(said.contains("voicemeeter: not decided"), "{said}");
        assert!(!said.contains("voicemeeter: allowed"));
    }

    #[test]
    fn requests_that_end_before_the_call_has_filed_them_all_do_not_finish_it() {
        let k = fresh("early");
        assert!(add(&k, "block-1", "early-a", "one").grouped());
        assert_eq!(settle("early-a", Settled::Declined, &open_all), None);
        assert!(add(&k, "block-1", "early-b", "two").grouped());
        // Sealing with the second still open sends nothing; its end then sends the one note.
        assert_eq!(seal(&k, &|id| id == "early-b"), None);
        let (_, said) = settle("early-b", Settled::Allowed, &open_all).unwrap();
        assert!(said.contains("one: declined") && said.contains("two: allowed"), "{said}");
    }

    #[test]
    fn a_group_whose_requests_all_ended_before_sealing_is_told_at_the_seal() {
        let k = fresh("sealed-late");
        assert!(add(&k, "block-1", "late-a", "one").grouped());
        settle("late-a", Settled::Vetoed, &open_all);
        let (_, said) = seal(&k, &open_all).expect("nothing is left open");
        assert!(said.contains("one: vetoed by the person"), "{said}");
    }

    #[test]
    fn a_withdrawn_request_does_not_hold_the_group_forever() {
        let k = fresh("withdrawn");
        assert!(add(&k, "block-1", "wd-a", "one").grouped());
        assert!(add(&k, "block-1", "wd-b", "two").grouped());
        seal(&k, &open_all);
        settle("wd-a", Settled::Allowed, &open_all);
        // `wd-b` is no longer open in the registry and has no outcome of its own.
        let (_, said) = settle("wd-a", Settled::Allowed, &|id| id != "wd-b").unwrap();
        assert!(said.contains("one: allowed") && said.contains("two: not decided"), "{said}");
    }

    #[test]
    fn a_failed_grant_says_why_without_allowing_it() {
        let k = fresh("failed");
        assert!(add(&k, "block-1", "f-a", "one").grouped());
        seal(&k, &open_all);
        let (_, said) = settle("f-a", Settled::Failed("Desktop driving is off".into()), &open_all).unwrap();
        assert!(said.contains("one: could not be allowed: Desktop driving is off"), "{said}");
    }

    #[test]
    fn a_group_is_bounded_and_one_agents_call_is_not_joined_by_anothers() {
        let k = fresh("bounded");
        for i in 0..MAX_MEMBERS {
            assert!(add(&k, "block-1", &format!("b-{i}"), "app").grouped(), "{i}");
        }
        assert_eq!(add(&k, "block-1", "b-extra", "app"), Added::No, "a ninth app is not added");
        // A different agent, pane or call has its own key.
        assert_ne!(key("a", "b", "c"), key("a", "b", "d"));
        assert_ne!(key("a", "b", "c"), key("x", "b", "c"));
        assert_ne!(key("a", "b", "c"), key("a", "y", "c"));
        // A sealed group takes no more.
        let s = fresh("closed");
        assert!(add(&s, "block-1", "c-a", "app").grouped());
        seal(&s, &|_| true);
        assert_eq!(add(&s, "block-1", "c-b", "app"), Added::No);
    }

    #[test]
    fn the_first_request_creates_the_group_and_the_rest_join_it() {
        let k = fresh("created");
        assert_eq!(add(&k, "block-1", "cr-a", "one"), Added::Created);
        assert_eq!(add(&k, "block-1", "cr-b", "two"), Added::Joined);
        flush(&k, &|_| true);
    }

    #[test]
    fn a_group_that_never_finishes_is_flushed_by_its_deadline_and_stops_holding_notes() {
        let k = fresh("stuck");
        assert!(add(&k, "block-1", "st-a", "one").grouped());
        assert!(add(&k, "block-1", "st-b", "two").grouped());
        // The call was never sealed; `one` was allowed, `two` is still open.
        settle("st-a", Settled::Allowed, &|_| true);
        assert!(is_member("st-b"));
        let (block, said) = flush(&k, &|id| id == "st-b").expect("the deadline ends it");
        assert_eq!(block, "block-1");
        assert!(said.contains("one: allowed"), "{said}");
        assert!(said.contains("two: still waiting for an answer; its own note will follow"), "{said}");
        // Its requests send their own notes again, and the slot is free.
        assert!(!is_member("st-b"));
        assert_eq!(flush(&k, &|_| true), None);
    }
}
