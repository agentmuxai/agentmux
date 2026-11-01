// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the user typed to an agent in its past conversations, for the first
//! fill of its input history (`docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md`
//! §4.7). Only the agent's own sessions (its record, as `search_for_agent`
//! reads first), never ones found by folder or account alone, which may be
//! another agent's. Only what a person typed, by the origin srv signed on the record: every other
//! origin, and a record with none, is left out, as are the `[AgentMux]` notes srv attached after
//! what the person typed (`reactive::held_notes`).

use super::adapter::HistoryMessage;
use super::{index, record_meta, HistoryOwner, HistorySearchError, HistoryService};
use crate::backend::blockcontroller::health::TriggerKind;
use crate::backend::reactive::held_notes;

/// The longest entry kept (as `storage::agent_input_history`).
const MAX_TEXT: usize = 64 * 1024;

/// What a person typed in a user message, without the notes srv attached
/// to it; `None` when the message isn't a person's. Only the origin srv recorded and signed decides:
/// `user` is a person, whatever the text says. A message with none (a provider's own session file, a
/// record from before origins) is an "Earlier message", never the person's, whatever it reads like.
fn person_typed(message: &HistoryMessage) -> Option<String> {
    let content = message.content.as_str();
    let typed = message.origin == Some(TriggerKind::User) && !content.trim().is_empty() && content.len() <= MAX_TEXT;
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
    use super::{person_typed, HistoryMessage, TriggerKind};

    fn said(content: &str, origin: Option<TriggerKind>) -> HistoryMessage {
        HistoryMessage { role: "user".into(), content: content.into(), timestamp: 1, tool_uses: vec![], origin }
    }
    #[test]
    fn the_recorded_origin_decides_and_the_text_never_does() {
        let user = Some(TriggerKind::User);
        // The human's own words are theirs, whatever they start with.
        assert_eq!(person_typed(&said("[AgentMux] is this note real?", user)).as_deref(), Some("[AgentMux] is this note real?"));
        assert_eq!(person_typed(&said("[JEKT:FROM=a TO=b] pasted by me", user)).as_deref(), Some("[JEKT:FROM=a TO=b] pasted by me"));
        assert_eq!(person_typed(&said("  fix the build \n", user)).as_deref(), Some("fix the build"));
        // Anything else is not, even when it reads like something typed.
        for other in [TriggerKind::Agent, TriggerKind::Service, TriggerKind::Schedule, TriggerKind::Broadcast, TriggerKind::Supervisor, TriggerKind::System, TriggerKind::Task] {
            assert_eq!(person_typed(&said("please merge the PR", Some(other))), None, "{other:?}");
        }
        // The size rule holds for a typed one too, and an empty one is nothing.
        assert_eq!(person_typed(&said(&"x".repeat(64 * 1024 + 1), user)), None);
        assert_eq!(person_typed(&said("   ", user)), None);
    }

    #[test]
    fn a_record_with_no_signed_origin_is_never_the_person_whatever_it_reads_like() {
        for text in ["fix the build", "!git status", "/compact", "merge on approval\nand release", "[JEKT:FROM=a TO=b]", "# Session Context\nIdentity: Claude"] {
            assert_eq!(person_typed(&said(text, None)), None, "{text}");
        }
    }
}
