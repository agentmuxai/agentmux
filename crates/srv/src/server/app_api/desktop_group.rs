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

/// Add the request filed for `app` to the group of this call, making the group if it is the first.
/// Refused when the group is full, sealed, or too many groups wait; the request then goes on as its own.
pub(crate) fn add(key: &str, block_id: &str, request_id: &str, app: &str) -> bool {
    let mut groups = GROUPS.lock().unwrap_or_else(|e| e.into_inner());
    let member = Member { request_id: request_id.to_string(), app: app.to_string(), outcome: None };
    if let Some(g) = groups.iter_mut().find(|g| g.key == key) {
        if g.sealed || g.members.len() >= MAX_MEMBERS || g.members.iter().any(|m| m.request_id == request_id) {
            return false;
        }
        g.members.push(member);
        return true;
    }
    if groups.len() >= MAX_GROUPS {
        return false;
    }
    groups.push(Group { key: key.to_string(), block_id: block_id.to_string(), members: vec![member], sealed: false });
    true
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
    Some((g.block_id.clone(), note(&g)))
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
fn line(m: &Member) -> String {
    let said = match &m.outcome {
        Some(Settled::Allowed) => "allowed, you may use it now".to_string(),
        Some(Settled::Failed(why)) => format!("could not be allowed: {}", crate::backend::reactive::sanitize::quote_in_system_note(why)),
        Some(Settled::Declined) => "declined".to_string(),
        Some(Settled::Vetoed) => "vetoed by the person".to_string(),
        Some(Settled::Ended) | None => "not decided (withdrawn or timed out)".to_string(),
    };
    format!("{}: {said}", m.app)
}

fn note(g: &Group) -> String {
    let lines: Vec<String> = g.members.iter().map(line).collect();
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
        assert!(add(&k, "block-1", "mixed-a", "cantabile"));
        assert!(add(&k, "block-1", "mixed-b", "voicemeeter"));
        assert!(add(&k, "block-1", "mixed-c", "notepad"));
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
        assert!(add(&k, "block-1", "alone-a", "cantabile"));
        assert!(add(&k, "block-1", "alone-b", "voicemeeter"));
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
        assert!(add(&k, "block-1", "early-a", "one"));
        assert_eq!(settle("early-a", Settled::Declined, &open_all), None);
        assert!(add(&k, "block-1", "early-b", "two"));
        // Sealing with the second still open sends nothing; its end then sends the one note.
        assert_eq!(seal(&k, &|id| id == "early-b"), None);
        let (_, said) = settle("early-b", Settled::Allowed, &open_all).unwrap();
        assert!(said.contains("one: declined") && said.contains("two: allowed"), "{said}");
    }

    #[test]
    fn a_group_whose_requests_all_ended_before_sealing_is_told_at_the_seal() {
        let k = fresh("sealed-late");
        assert!(add(&k, "block-1", "late-a", "one"));
        settle("late-a", Settled::Vetoed, &open_all);
        let (_, said) = seal(&k, &open_all).expect("nothing is left open");
        assert!(said.contains("one: vetoed by the person"), "{said}");
    }

    #[test]
    fn a_withdrawn_request_does_not_hold_the_group_forever() {
        let k = fresh("withdrawn");
        assert!(add(&k, "block-1", "wd-a", "one"));
        assert!(add(&k, "block-1", "wd-b", "two"));
        seal(&k, &open_all);
        settle("wd-a", Settled::Allowed, &open_all);
        // `wd-b` is no longer open in the registry and has no outcome of its own.
        let (_, said) = settle("wd-a", Settled::Allowed, &|id| id != "wd-b").unwrap();
        assert!(said.contains("one: allowed") && said.contains("two: not decided"), "{said}");
    }

    #[test]
    fn a_failed_grant_says_why_without_allowing_it() {
        let k = fresh("failed");
        assert!(add(&k, "block-1", "f-a", "one"));
        seal(&k, &open_all);
        let (_, said) = settle("f-a", Settled::Failed("Desktop driving is off".into()), &open_all).unwrap();
        assert!(said.contains("one: could not be allowed: Desktop driving is off"), "{said}");
    }

    #[test]
    fn a_group_is_bounded_and_one_agents_call_is_not_joined_by_anothers() {
        let k = fresh("bounded");
        for i in 0..MAX_MEMBERS {
            assert!(add(&k, "block-1", &format!("b-{i}"), "app"), "{i}");
        }
        assert!(!add(&k, "block-1", "b-extra", "app"), "a ninth app is not added");
        // A different agent, pane or call has its own key.
        assert_ne!(key("a", "b", "c"), key("a", "b", "d"));
        assert_ne!(key("a", "b", "c"), key("x", "b", "c"));
        assert_ne!(key("a", "b", "c"), key("a", "y", "c"));
        // A sealed group takes no more.
        let s = fresh("closed");
        assert!(add(&s, "block-1", "c-a", "app"));
        seal(&s, &|_| true);
        assert!(!add(&s, "block-1", "c-b", "app"));
    }
}
