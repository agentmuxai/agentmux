// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the user typed to an agent in its past conversations, for the first
//! fill of its input history (`docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md`
//! §4.7). Only the agent's own sessions (its record, as `search_for_agent`
//! reads first), never ones found by folder or account alone, which may be
//! another agent's. Only what a person typed: a user message that is a jekt,
//! an `[AgentMux]` note, a broadcast's header, a hook's or the CLI's own
//! injected text is left out, as are the `[AgentMux]` notes srv attached after
//! what the person typed (`reactive::held_notes`).

use super::adapter::HistoryMessage;
use super::{index, record_meta, HistoryOwner, HistorySearchError, HistoryService};
use crate::backend::blockcontroller::health::TriggerKind;
use crate::backend::reactive::held_notes;

/// The longest entry kept (as `storage::agent_input_history`).
const MAX_TEXT: usize = 64 * 1024;

/// Text the CLI or AgentMux put in a user turn, not the person: it starts
/// with one of these.
const INJECTED: [&str; 15] = [
    "[JEKT:",
    "[AgentMux]",
    "[BULLETIN:",
    "[BROADCAST:",
    "<system-reminder>",
    "<command-name>",
    "<command-message>",
    "<command-args>",
    "<local-command-",
    "<task-notification>",
    "<user-prompt-submit-hook>",
    "<bash-input>",
    "<bash-stdout>",
    "Caveat:",
    "This session is being continued",
];

/// Whether a user message's text is something a person typed.
pub(crate) fn typed_by_a_person(content: &str) -> bool {
    let text = content.trim_start();
    // A note or jekt that arrived late carries "(sent 14:30, delivered 14:41)" first.
    let after_delay = text
        .strip_prefix("(sent ")
        .and_then(|rest| rest.split_once(')'))
        .map(|(_, rest)| rest.trim_start())
        .unwrap_or(text);
    !text.is_empty()
        && text.len() <= MAX_TEXT
        && !INJECTED.iter().any(|p| text.starts_with(p) || after_delay.starts_with(p))
}

/// What a person typed in a user message, without the notes srv attached
/// to it; `None` when the message isn't a person's. The origin srv recorded and signed decides when
/// the message has one: only `user` is a person, whatever the text says. A message with none (a
/// provider's own session file, a record from before origins) is judged by its text, as before.
fn person_typed(message: &HistoryMessage) -> Option<String> {
    let content = message.content.as_str();
    let typed = match message.origin {
        Some(kind) => kind == TriggerKind::User && !content.trim().is_empty() && content.len() <= MAX_TEXT,
        None => typed_by_a_person(content),
    };
    typed.then(|| held_notes::strip_attached(content).trim().to_string())
}

impl HistoryService {
    /// Every message the user typed to the agent `owner`, oldest first, with
    /// when it was sent (unix ms); a repeat of the one before is left out, and
    /// only the newest `max` are kept.
    pub fn typed_by_user(&self, owner: &HistoryOwner, max: usize) -> Result<Vec<(i64, String)>, HistorySearchError> {
        self.refresh_for_search()?;
        let (ledger, _problems) = self.records.sessions_of(&owner.definition_id);
        let candidates: Vec<index::SearchCandidate> = ledger
            .into_iter()
            .map(|session| match self.index.get_meta(&session.session_id) {
                Some(meta) => index::SearchCandidate {
                    meta,
                    source: index::Source::Transcript,
                    attribution: index::Attribution::AgentRecord,
                },
                None => index::SearchCandidate {
                    meta: record_meta(&session),
                    source: index::Source::Record(session),
                    attribution: index::Attribution::AgentRecord,
                },
            })
            .collect();
        let mut typed: Vec<(i64, String)> = Vec::new();
        for candidate in &candidates {
            let Ok(Some(session)) = self.load(candidate) else { continue };
            typed.extend(
                session
                    .messages
                    .into_iter()
                    .filter(|m| m.role == "user")
                    .filter_map(|m| Some((m.timestamp, person_typed(&m)?))),
            );
        }
        typed.sort_by_key(|(at, _)| *at);
        typed.dedup_by(|b, a| a.1 == b.1);
        let skip = typed.len().saturating_sub(max);
        Ok(typed.split_off(skip))
    }
}

#[cfg(test)]
mod tests {
    use super::{person_typed, typed_by_a_person, HistoryMessage, TriggerKind};

    fn said(content: &str, origin: Option<TriggerKind>) -> HistoryMessage {
        HistoryMessage { role: "user".into(), content: content.into(), timestamp: 1, tool_uses: vec![], origin }
    }
    fn typed(content: &str) -> Option<String> {
        person_typed(&said(content, None))
    }

    #[test]
    fn only_what_a_person_typed_is_kept() {
        for typed in ["fix the build", "  /compact  ", "!git status", "merge on approval\nand release"] {
            assert!(typed_by_a_person(typed), "{typed}");
        }
        for injected in [
            "[JEKT:FROM=korp TO=QTest TIER=coord ...]",
            "[AgentMux] AgentMux was upgraded from 0.59.31 to 0.59.32.",
            "(sent 14:30, delivered 14:41) [AgentMux] Cloud messages resumed",
            "(sent 14:30, delivered 14:41) [JEKT:FROM=a]",
            "[BROADCAST:FROM=user VIA=swarm TO=QTest RECIPIENTS=3 MSGID=1 TS=2]\nmerge",
            "<system-reminder>\nnote\n</system-reminder>",
            "<command-name>/compact</command-name>",
            "<local-command-stdout>ok</local-command-stdout>",
            "Caveat: The messages below were generated by the user while running local commands.",
            "This session is being continued from a previous conversation",
            "   ",
        ] {
            assert!(!typed_by_a_person(injected), "{injected}");
        }
        assert!(!typed_by_a_person(&"x".repeat(64 * 1024 + 1)), "a pasted file isn't history");
    }

    #[test]
    fn agentmux_text_a_person_pasted_is_kept() {
        let pasted = "explain this:\n\n[AgentMux] Cloud messages are paused since 14:22 (MuxBus needs a sign-in).";
        assert_eq!(typed(pasted).as_deref(), Some(pasted));
        assert_eq!(typed("  fix the build \n").as_deref(), Some("fix the build"));
        assert_eq!(typed("[AgentMux] AgentMux was upgraded"), None);
    }

    #[test]
    fn the_recorded_origin_decides_and_the_text_only_when_there_is_none() {
        let user = Some(TriggerKind::User);
        // The human's own words are theirs, whatever they start with.
        assert_eq!(person_typed(&said("[AgentMux] is this note real?", user)).as_deref(), Some("[AgentMux] is this note real?"));
        assert_eq!(person_typed(&said("  fix the build \n", user)).as_deref(), Some("fix the build"));
        // Anything else is not, even when it reads like something typed.
        for other in [TriggerKind::Agent, TriggerKind::Service, TriggerKind::Schedule, TriggerKind::Broadcast, TriggerKind::Supervisor, TriggerKind::System, TriggerKind::Task] {
            assert_eq!(person_typed(&said("please merge the PR", Some(other))), None, "{other:?}");
        }
        // The size rule holds for a typed one too, and an empty one is nothing.
        assert_eq!(person_typed(&said(&"x".repeat(64 * 1024 + 1), user)), None);
        assert_eq!(person_typed(&said("   ", user)), None);
        // No origin: the text decides, as for a provider's own session file.
        assert_eq!(typed("please merge the PR").as_deref(), Some("please merge the PR"));
        assert_eq!(typed("[JEKT:FROM=a TO=b]"), None);
    }
}
