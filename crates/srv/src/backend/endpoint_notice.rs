// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The notice that goes with pointing an agent at another model server (`Agents env_set` of
//! `ANTHROPIC_BASE_URL` and the variables that go with it, `backend::agent_env`), and the window in
//! which anyone can undo it.
//!
//! The change is written at once and applies at the agent's next launch. For [`WINDOW_MS`] after it,
//! a card in the agent's pane and a line in My Agents say who pointed it where, with an Undo that
//! puts the environment back as it was. Past the window the change stands and is changed like any
//! other, in My Agents or by `env_set`. The notice holds the text from before server-side only: it can
//! hold the operator's own variables, so the event carries the address and model names and never it.

use serde::Serialize;
use std::sync::Mutex;

/// How long an Undo is offered.
pub const WINDOW_MS: i64 = 30_000;

/// `agent:endpoint-notice`: an agent's model server was changed, or the change was undone.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct EndpointNoticeEvent {
    pub id: String,
    pub agent_id: String,
    pub agent_slug: String,
    pub agent_name: String,
    /// The agent that made the change (its display name), as srv's record has it.
    pub by: String,
    /// The server the agent is pointed at now, if it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub base_url: Option<String>,
    /// The model it was given, if one was named.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<String>,
    /// The agent was pointed back at Anthropic: no server of its own any more.
    pub cleared: bool,
    /// `open`, or `undone` once the change was put back.
    pub state: String,
    #[ts(type = "number")]
    pub window_ms: i64,
    /// When the Undo stops being offered (epoch ms).
    #[ts(type = "number")]
    pub expires_at: i64,
}

struct Held {
    event: EndpointNoticeEvent,
    /// The environment before the change, and after it: Undo needs the first, and refuses when the
    /// second is no longer what is stored.
    previous: String,
    applied: String,
}

static HELD: Mutex<Vec<Held>> = Mutex::new(Vec::new());

/// What the change leaves the agent pointed at, from the new environment text.
pub struct Pointed {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub cleared: bool,
}

/// Where `new` points the agent, against `old`.
pub fn pointed(old: &str, new: &str) -> Pointed {
    let (old, new) = (super::agent_env::effective(old), super::agent_env::effective(new));
    let url = new.get("ANTHROPIC_BASE_URL").map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    let model = new
        .get("ANTHROPIC_MODEL")
        .or_else(|| new.get("ANTHROPIC_DEFAULT_SONNET_MODEL"))
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    let had_url = old.get("ANTHROPIC_BASE_URL").is_some_and(|u| !u.trim().is_empty());
    Pointed { cleared: url.is_none() && had_url, base_url: url, model }
}

/// Opens the notice for a change and keeps what Undo needs. Returns the event to publish.
pub fn open(agent: (&str, &str, &str), by: &str, pointed: Pointed, previous: &str, applied: &str, now: i64) -> EndpointNoticeEvent {
    let event = EndpointNoticeEvent {
        id: uuid::Uuid::new_v4().to_string(),
        agent_id: agent.0.to_string(),
        agent_slug: agent.1.to_string(),
        agent_name: agent.2.to_string(),
        by: by.to_string(),
        base_url: pointed.base_url,
        model: pointed.model,
        cleared: pointed.cleared,
        state: "open".to_string(),
        window_ms: WINDOW_MS,
        expires_at: now + WINDOW_MS,
    };
    let mut held = HELD.lock().unwrap_or_else(|p| p.into_inner());
    held.retain(|h| h.event.expires_at > now);
    held.push(Held { event: event.clone(), previous: previous.to_string(), applied: applied.to_string() });
    event
}

/// The agent a held notice is for, if it is still held.
pub fn agent_id_of(id: &str) -> Option<String> {
    HELD.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|h| h.event.id == id).map(|h| h.event.agent_id.clone())
}

/// Why an Undo was not done.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No such notice, or it was already undone.
    Unknown,
    /// The window is over: the change stands.
    Expired,
    /// The environment was changed again since: putting the old text back would undo more than this change.
    ChangedSince,
}

impl Refusal {
    pub fn words(&self) -> &'static str {
        match self {
            Refusal::Unknown => "that change was already undone, or srv no longer has it",
            Refusal::Expired => "the undo window is over; change the environment in My Agents",
            Refusal::ChangedSince => "the environment was changed again since; change it in My Agents",
        }
    }
}

/// Checks the notice `id` for an Undo: the text to put back and the event that says it was undone.
/// `current` is the environment as stored now. The notice stays held until [`finish_undo`], so a write
/// that fails can be tried again inside the window.
pub fn check_for_undo(id: &str, current: &str, now: i64) -> Result<(String, EndpointNoticeEvent), Refusal> {
    let mut held = HELD.lock().unwrap_or_else(|p| p.into_inner());
    let at = held.iter().position(|h| h.event.id == id).ok_or(Refusal::Unknown)?;
    if now > held[at].event.expires_at {
        held.remove(at);
        return Err(Refusal::Expired);
    }
    if held[at].applied != current {
        return Err(Refusal::ChangedSince);
    }
    let mut event = held[at].event.clone();
    event.state = "undone".to_string();
    Ok((held[at].previous.clone(), event))
}

/// The environment was put back: the notice cannot be undone again.
pub fn finish_undo(id: &str) {
    HELD.lock().unwrap_or_else(|p| p.into_inner()).retain(|h| h.event.id != id);
}

#[cfg(test)]
fn take_for_undo(id: &str, current: &str, now: i64) -> Result<(String, EndpointNoticeEvent), Refusal> {
    let taken = check_for_undo(id, current, now)?;
    finish_undo(id);
    Ok(taken)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEFORE: &str = "TZ=UTC\nOPERATOR_NOTE=keep\n";
    const AFTER: &str = "TZ=UTC\nOPERATOR_NOTE=keep\nANTHROPIC_BASE_URL=http://192.0.2.68:11434\nANTHROPIC_MODEL=flint:40b-a2b\nANTHROPIC_AUTH_TOKEN=ollama\n";

    fn opened(now: i64) -> EndpointNoticeEvent {
        open(("a1", "lolly", "Lolly"), "AgentY", pointed(BEFORE, AFTER), BEFORE, AFTER, now)
    }

    #[test]
    fn the_event_names_the_server_and_model_and_never_a_key_or_the_old_text() {
        let event = opened(1_000);
        assert_eq!(event.base_url.as_deref(), Some("http://192.0.2.68:11434"));
        assert_eq!(event.model.as_deref(), Some("flint:40b-a2b"));
        assert_eq!((event.by.as_str(), event.state.as_str(), event.expires_at), ("AgentY", "open", 1_000 + WINDOW_MS));
        let wire = serde_json::to_string(&event).unwrap();
        assert!(!wire.contains("ollama") && !wire.contains("OPERATOR_NOTE"), "{wire}");
    }

    #[test]
    fn pointing_back_at_anthropic_is_a_clear() {
        let p = pointed(AFTER, BEFORE);
        assert!(p.cleared);
        assert_eq!(p.base_url, None);
        assert!(!pointed(BEFORE, AFTER).cleared);
    }

    #[test]
    fn undo_in_the_window_gives_the_old_text_back_once() {
        let event = opened(1_000);
        let (text, undone) = take_for_undo(&event.id, AFTER, 1_000 + WINDOW_MS).unwrap();
        assert_eq!(text, BEFORE);
        assert_eq!(undone.state, "undone");
        assert_eq!(take_for_undo(&event.id, BEFORE, 2_000), Err(Refusal::Unknown), "once");
    }

    #[test]
    fn a_notice_stays_until_the_undo_was_written() {
        let event = opened(1_000);
        assert!(check_for_undo(&event.id, AFTER, 2_000).is_ok());
        assert!(check_for_undo(&event.id, AFTER, 2_001).is_ok(), "a failed write can be tried again");
        finish_undo(&event.id);
        assert_eq!(check_for_undo(&event.id, AFTER, 2_002), Err(Refusal::Unknown));
    }

    #[test]
    fn undo_after_the_window_or_after_a_further_change_is_refused() {
        let late = opened(1_000);
        assert_eq!(take_for_undo(&late.id, AFTER, 1_000 + WINDOW_MS + 1), Err(Refusal::Expired));
        let changed = opened(1_000);
        assert_eq!(take_for_undo(&changed.id, "TZ=UTC\n", 2_000), Err(Refusal::ChangedSince));
        // Refused for being changed, it is still there once the text matches again.
        assert!(take_for_undo(&changed.id, AFTER, 2_000).is_ok());
        assert_eq!(take_for_undo("no-such-notice", AFTER, 2_000), Err(Refusal::Unknown));
    }
}
