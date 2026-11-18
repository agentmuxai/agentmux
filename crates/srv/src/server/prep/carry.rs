// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where the handoff an agent left goes once its session has restarted (spec §3.4). A persistent
//! Claude pane reads it in its own spawn (`carry_prep_handoff`), as the first section of the packet
//! or a held note. A pane of any other controller (a subprocess, ACP or app-server agent) has no
//! such step, so the action that restarted it hands the handoff over here, as a note held for the
//! pane's next message.

use crate::backend::blockcontroller::BLOCK_CONTROLLER_PERSISTENT;
use crate::backend::continuity_handoff::{self, NOTE_KEY};

/// What reads a restarted pane's handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reader {
    /// Its own spawn does (persistent Claude, or a pane that is not up yet and will take it when it is).
    Spawn,
    /// Nothing does: it is held as a note for the pane's next message.
    Note,
}

fn reader_of(block: &str) -> Reader {
    match crate::backend::blockcontroller::get_controller(block) {
        Some(ctrl) if ctrl.controller_type() != BLOCK_CONTROLLER_PERSISTENT => Reader::Note,
        _ => Reader::Spawn,
    }
}

/// After the agent `def_id`'s panes `restarted` came up again: hold its handoff as a note for each
/// pane whose controller has no spawn step that reads it. Left where it is (for the spawn to read)
/// when any pane of the agent is read by its spawn. `true` when it was held.
pub fn hold_for_other_panes(def_id: &str, restarted: &[&str]) -> bool {
    hold_where(def_id, restarted, reader_of)
}

fn hold_where(def_id: &str, restarted: &[&str], reader: impl Fn(&str) -> Reader) -> bool {
    let readers: Vec<Reader> = restarted.iter().map(|b| reader(b)).collect();
    if readers.is_empty() || readers.contains(&Reader::Spawn) {
        return false;
    }
    let Some(handoff) = continuity_handoff::take(def_id) else { return false };
    tracing::info!(def_id, panes = restarted.len(), "continuity: holding the handoff the agent left as a note, its controller has no spawn step that reads it");
    let note = handoff.note();
    for block in restarted {
        crate::backend::reactive::held_notes::hold(block, NOTE_KEY, &note);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::continuity_handoff::Handoff;
    use crate::backend::reactive::held_notes;
    use std::sync::Arc;

    fn handoff(text: &str) -> Handoff {
        Handoff { prep_id: "p-1".into(), at_ms: agentmux_common::time::now_ms(), actor: "operator".into(), action: "account_switch".into(), text: text.into() }
    }

    fn store_installed() {
        crate::backend::agent_session::set_global_transcript_store(Arc::new(crate::backend::storage::filestore::FileStore::open_in_memory().unwrap()));
    }

    #[test]
    fn a_pane_with_no_spawn_step_gets_the_handoff_as_a_held_note_and_it_is_taken() {
        store_installed();
        continuity_handoff::store("carry-def-a", &handoff("In flight: PR 7")).unwrap();
        assert!(hold_where("carry-def-a", &["carry-blk-a1", "carry-blk-a2"], |_| Reader::Note));
        for block in ["carry-blk-a1", "carry-blk-a2"] {
            let (message, _held) = held_notes::attach(block, "hello");
            assert!(message.contains("In flight: PR 7"), "{block}: {message}");
        }
        assert!(continuity_handoff::take("carry-def-a").is_none(), "read once");
    }

    #[test]
    fn a_pane_whose_spawn_reads_it_is_left_the_handoff() {
        store_installed();
        continuity_handoff::store("carry-def-b", &handoff("Waiting on CI")).unwrap();
        let reader = |b: &str| if b == "carry-blk-b1" { Reader::Spawn } else { Reader::Note };
        assert!(!hold_where("carry-def-b", &["carry-blk-b1", "carry-blk-b2"], reader));
        assert_eq!(continuity_handoff::take("carry-def-b").unwrap().text, "Waiting on CI");
        assert!(!held_notes::attach("carry-blk-b2", "hello").0.contains("Waiting on CI"));
    }

    #[test]
    fn nothing_is_held_when_the_agent_left_no_handoff_or_no_pane_restarted() {
        store_installed();
        assert!(!hold_where("carry-def-c", &["carry-blk-c"], |_| Reader::Note));
        continuity_handoff::store("carry-def-d", &handoff("x")).unwrap();
        assert!(!hold_where("carry-def-d", &[], |_| Reader::Note));
        assert!(continuity_handoff::take("carry-def-d").is_some(), "left for whoever restarts next");
    }
}
