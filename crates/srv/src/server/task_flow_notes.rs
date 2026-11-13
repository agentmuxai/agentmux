// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tells the Atlas map what happened to a task: `backend::task_flow`. Names, ids and numbers only; nothing here reads
//! a task's title, payload or result.

use serde_json::Value;

use super::AppState;
use crate::backend::message_flow::MessageFlowEnd;
use crate::backend::task_flow::{self, Seen, TaskFlowKind};

/// An agent's end of a line: `slug@machine` names an agent of another computer, any other name one of this computer.
/// The operator, the app itself and nobody have no end.
fn end_of(name: &str, here: &str) -> Option<MessageFlowEnd> {
    let name = name.trim();
    if name.is_empty() || ["operator", "agentmux", "system"].iter().any(|n| name.eq_ignore_ascii_case(n)) {
        return None;
    }
    Some(match name.split_once('@') {
        Some((agent, host)) if !agent.is_empty() && !host.is_empty() => MessageFlowEnd { agent: agent.to_string(), host: Some(host.to_lowercase()) },
        _ => MessageFlowEnd { agent: name.to_string(), host: Some(here.to_string()) },
    })
}

/// Reports that `by` did `kind` to the task `work_id` on this computer's board, for the agent `to` (empty: for anyone).
pub(crate) fn note(state: &AppState, work_id: &str, kind: TaskFlowKind, by: &str, to: &str) {
    let Ok(Some((board, number))) = state.work_store.work_task_board(work_id) else { return };
    let here = crate::backend::reactive::registry::local_host_label();
    task_flow::note(&Seen { number, board: board.slug, board_id: board.id, host: here.clone(), by: end_of(by, &here), to: end_of(to, &here), kind });
}

/// The lines for a task `slug` filed on the board of `peer_host` (`host` of a Tasks call), from the answer that machine
/// gave: `{num, board}`, and `{build_num}` for a spec and its build. The board's id is not known here, so a line is kept
/// under the board's slug and a reader that is an agent never gets it.
fn remote_filed(slug: &str, here: &str, peer_host: &str, target: &str, answer: &Value) -> Vec<Seen> {
    let peer = peer_host.trim().to_lowercase();
    let board = answer.get("board").and_then(Value::as_str).unwrap_or_default();
    let to = Some(target.trim()).filter(|t| !t.is_empty()).map(|t| MessageFlowEnd { agent: t.to_string(), host: Some(peer.clone()) });
    ["num", "build_num"]
        .iter()
        .filter_map(|key| answer.get(*key).and_then(Value::as_i64))
        .map(|number| Seen {
            number,
            board: board.to_string(),
            board_id: format!("remote:{peer}:{board}"),
            host: peer.clone(),
            by: Some(MessageFlowEnd { agent: slug.to_string(), host: Some(here.to_string()) }),
            to: to.clone(),
            kind: TaskFlowKind::Filed,
        })
        .collect()
}

/// Reports a task `slug` filed on another computer's board, from the answer that machine gave. That machine reports it
/// too; a reader joins the two by the task's host and number.
pub(crate) fn note_remote_filed(slug: &str, peer_host: &str, target: &str, answer: &Value) {
    let here = crate::backend::reactive::registry::local_host_label();
    remote_filed(slug, &here, peer_host, target, answer).iter().for_each(task_flow::note);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_of_this_computer_or_another_has_an_end_and_the_operator_has_none() {
        let end = |n| end_of(n, "charlie").map(|e| (e.agent, e.host));
        assert_eq!(end("maricon-2"), Some(("maricon-2".into(), Some("charlie".into()))));
        assert_eq!(end("agenty@Narko"), Some(("agenty".into(), Some("narko".into()))));
        assert_eq!((end("operator"), end("Operator"), end("agentmux"), end(" "), end("")), (None, None, None, None, None));
    }

    #[test]
    fn a_task_filed_on_another_board_is_reported_by_that_boards_host_and_number() {
        let answer = serde_json::json!({ "id": "x", "num": 12, "board": "work", "build_num": 13 });
        let lines = remote_filed("maricon-2", "charlie", "Narko ", "agenty", &answer);
        let seen: Vec<_> = lines.iter().map(|l| (l.number, l.host.as_str(), l.board.as_str(), l.kind)).collect();
        assert_eq!(seen, [(12, "narko", "work", TaskFlowKind::Filed), (13, "narko", "work", TaskFlowKind::Filed)]);
        let by = lines[0].by.as_ref().unwrap();
        assert_eq!((by.agent.as_str(), by.host.as_deref()), ("maricon-2", Some("charlie")));
        let to = lines[0].to.as_ref().unwrap();
        assert_eq!((to.agent.as_str(), to.host.as_deref()), ("agenty", Some("narko")));
    }

    #[test]
    fn an_answer_with_no_number_reports_nothing_and_a_task_for_anyone_has_no_receiver() {
        assert!(remote_filed("a", "x", "narko", "b", &serde_json::json!({ "error": "no" })).is_empty());
        assert!(remote_filed("a", "x", "narko", " ", &serde_json::json!({ "num": 1, "board": "w" }))[0].to.is_none());
    }
}
