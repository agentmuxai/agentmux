// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The two kinds of message the persistent controller holds before it writes
//! them to the process: the human path's queue around a spawn
//! ([`QueuedMessage`]) and the deliveries waiting for the process to take a
//! write ([`Deferred`]). Each carries where the message came from, for the
//! record written when it is delivered.

use crate::backend::message_origin::RecordOrigin;

/// A single entry in `pending_send_messages`: the formatted stdin JSON
/// payload plus whether it's already been persisted to the blockfile.
/// codex P2 on PR #2360 (sixth review pass, round 7): a stale-resume
/// retry's redelivery has already been correctly persisted on its
/// original (failed) attempt — see `retry_after_resume_failure`'s own doc
/// comment — but the drain used to persist EVERY message it delivers
/// unconditionally, double-persisting a replayed retry batch into the
/// blockfile transcript.
#[derive(Clone, Debug)]
pub(super) struct QueuedMessage {
    /// Queue identity (issue #2365): assigned once from
    /// `PersistentInner::next_message_seq` at first enqueue and preserved
    /// verbatim across redelivery (`QueuedRetryEntry::seq` carries it
    /// through a stale-resume retry batch), so dedup / own-message /
    /// seed checks are exact identity — two genuinely different messages
    /// with identical text can never be conflated.
    pub(super) seq: u64,
    pub(super) json_str: String,
    pub(super) already_persisted: bool,
    /// Where the message came from, for the record written when it is
    /// delivered. `None` only for an `already_persisted` entry, whose record
    /// was written on its first attempt.
    pub(super) record: Option<RecordOrigin>,
}

impl QueuedMessage {
    pub(super) fn fresh(seq: u64, json_str: String, record: RecordOrigin) -> Self {
        Self { seq, json_str, already_persisted: false, record: Some(record) }
    }

    pub(super) fn already_persisted(seq: u64, json_str: String) -> Self {
        Self { seq, json_str, already_persisted: true, record: None }
    }
}

impl PartialEq<str> for QueuedMessage {
    fn eq(&self, other: &str) -> bool {
        self.json_str == other
    }
}

impl PartialEq<&str> for QueuedMessage {
    fn eq(&self, other: &&str) -> bool {
        self.json_str == *other
    }
}

impl PartialEq<String> for QueuedMessage {
    fn eq(&self, other: &String) -> bool {
        self.json_str == *other
    }
}

/// A message waiting in `PersistentInner::deferred_deliveries`: the encoded
/// stream-json stdin `line` and where the message came from, which the record
/// of it carries when it is written (`message_origin`).
#[derive(Clone, Debug)]
pub(super) struct Deferred {
    pub(super) line: String,
    pub(super) record: RecordOrigin,
}

impl std::ops::Deref for Deferred {
    type Target = str;
    fn deref(&self) -> &str {
        &self.line
    }
}

#[cfg(test)]
impl From<String> for Deferred {
    /// A test's queued line, from the human.
    fn from(line: String) -> Self {
        Deferred {
            line,
            record: RecordOrigin::new(
                crate::backend::message_origin::MessageOrigin::user(),
            ),
        }
    }
}
