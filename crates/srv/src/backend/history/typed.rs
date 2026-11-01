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
            .map(|session| match (self.index.get_meta(&session.session_id), session.ranges.is_empty()) {
                // The record carries the origin srv signed; a provider's transcript carries none, so a
                // session the record holds is read from the record even when its transcript is indexed.
                (meta, false) => index::SearchCandidate {
                    meta: meta.unwrap_or_else(|| record_meta(&session)),
                    source: index::Source::Record(session),
                    attribution: index::Attribution::AgentRecord,
                },
                (Some(meta), true) => index::SearchCandidate {
                    meta,
                    source: index::Source::Transcript,
                    attribution: index::Attribution::AgentRecord,
                },
                (None, true) => index::SearchCandidate {
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
    /// A session whose provider transcript is indexed is still read from the agent's record, which holds the
    /// origin srv signed: the transcript has none, so reading it would leave the input history empty.
    #[test]
    fn an_indexed_session_is_read_from_the_signed_record_not_the_transcript() {
        use crate::backend::history::record::fixtures::{append, assistant, init, store, zone, UID};
        use crate::backend::history::{adapter::HistoryAdapter, claude_adapter::ClaudeHistoryAdapter, index::SessionIndex, record::AgentRecords, HistoryOwner, HistoryService};
        use crate::backend::transcript::origin_sig::sign_line;
        const CWD: &str = "/work/agent-a";
        let tmp = tempfile::TempDir::new().unwrap();
        let project = tmp.path().join("providers").join("claude").join("projects").join("-work-agent-a");
        std::fs::create_dir_all(&project).unwrap();
        let user = |text: &str| format!(r#"{{"type":"user","message":{{"role":"user","content":"{text}"}},"timestamp":"2026-09-24T10:00:00Z","cwd":"{CWD}"}}"#);
        std::fs::write(project.join("s1.jsonl"), format!("{}\n", user("from the provider file"))).unwrap();
        let signed = |kind: &str, text: &str| {
            let line = format!(r#"{{"type":"user","message":{{"role":"user","content":"{text}"}},"amx":{{"v":1,"origin":{{"kind":"{kind}"}}}}}}"#);
            String::from_utf8(sign_line(UID, line.as_bytes(), 1_700_000_000_000)).unwrap()
        };
        let records = store();
        append(&records, &zone(), &[init("s1", CWD), signed("user", "typed by the person"), signed("agent", "sent by an agent"), assistant("s1", "answer")]);
        let adapters: Vec<Box<dyn HistoryAdapter>> = vec![Box::new(ClaudeHistoryAdapter::with_roots(None, Some(tmp.path().to_path_buf()), None))];
        let service = HistoryService::from_index_and_records(SessionIndex::with_isolated_roots(adapters, vec![]), AgentRecords::new(Some(records)));
        let owner = HistoryOwner { definition_id: UID.into(), working_directory: Some(CWD.into()) };
        let typed: Vec<String> = service.typed_by_user(&owner, 10).unwrap().into_iter().map(|(_, t)| t).collect();
        assert_eq!(typed, vec!["typed by the person".to_string()]);
    }
}
