// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::rpc_types::AuditChannelRows;
use serde_json::json;

fn row(id: &str, ts_ms: u64) -> AuditEntry {
    serde_json::from_value(json!({
        "id": id, "ts_ms": ts_ms, "kind": "jekt", "summary": "a message",
        "links": { "pane": { "block_id": "b-1" }, "history": { "agent_id": "alpha", "at_ms": 1 }, "task": { "num": 7, "task_ref": "#7" } },
        "from": { "agent_id": "alpha", "links": { "pane": { "block_id": "b-2" } } },
        "provenance": { "host": "their-own-name", "channel": "stable" }
    }))
    .unwrap()
}

fn answer(own: Vec<AuditEntry>, other: Vec<AuditChannelRows>, next_cursor: Option<&str>) -> AuditQueryResult {
    AuditQueryResult {
        entries: own,
        next_cursor: next_cursor.map(str::to_string),
        channel: Some("stable".into()),
        other_channels: Some(other).filter(|o| !o.is_empty()),
        ..Default::default()
    }
}

#[test]
fn a_computer_is_asked_the_filter_for_its_first_page_of_every_channel() {
    let req = AuditQueryReq {
        agent_id: Some("alpha".into()),
        text: Some("x".into()),
        cursor: Some("5|1|a|-".into()),
        totals: Some(true),
        scope: Some("lan".into()),
        limit: Some(200),
        ..Default::default()
    };
    let sent = request_for_peer(&req);
    assert_eq!((sent.agent_id.as_deref(), sent.text.as_deref()), (Some("alpha"), Some("x")));
    // Its paging and totals are this srv's; `lan` would make the other computer ask its own peers in turn.
    assert_eq!((sent.cursor, sent.totals, sent.scope.as_deref()), (None, None, Some("machine")));
    assert_eq!(sent.limit, Some(PEER_LIMIT), "a large page is cut to what one computer answers with");
    assert_eq!(request_for_peer(&AuditQueryReq::default()).limit, Some(50));
}

#[test]
fn its_rows_are_one_list_by_time_with_the_computers_own_name_and_no_local_link() {
    let other = vec![AuditChannelRows { channel: "dev".into(), entries: vec![row("dev:o2", 20)], more: true }];
    let rows = rows_of("beta", answer(vec![row("j3", 30), row("j1", 10)], other, None));
    assert_eq!(rows.host, "beta");
    assert_eq!(rows.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["beta/j3", "beta/dev:o2", "beta/j1"]);
    assert!(rows.more, "its other channel holds older rows");
    assert!(rows.note.is_none());
    for e in &rows.entries {
        assert_eq!(e.provenance.as_ref().and_then(|p| p.host.as_deref()), Some("beta"), "the computer asked, not the name it wrote");
        assert_eq!(e.links, AuditLinks::default());
        assert_eq!(e.from.as_ref().unwrap().links, AuditLinks::default());
    }
}

#[test]
fn a_dialogs_text_and_the_answer_given_stay_on_the_computer_that_wrote_them() {
    let mut sensitive = row("j2", 20);
    sensitive.kind = "action".into();
    sensitive.prompt = Some("Allow the agent to read a file?".into());
    sensitive.answer = Some("yes, this one".into());
    sensitive.level = Some("high".into());
    let other = vec![AuditChannelRows { channel: "dev".into(), entries: vec![sensitive.clone()], more: false }];
    let rows = rows_of("beta", answer(vec![sensitive], other, None));
    assert_eq!(rows.entries.len(), 2);
    for e in &rows.entries {
        assert_eq!((e.prompt.as_deref(), e.answer.as_deref()), (None, None));
        assert_eq!(e.level.as_deref(), Some("high"), "how risky it was is not text a person typed");
    }
    // The answering computer removes it too, so it never leaves.
    let mut sent = [row("j3", 30)];
    sent[0].prompt = Some("text".into());
    sent[0].answer = Some("value".into());
    without_local_links(&mut sent);
    assert_eq!((sent[0].prompt.as_deref(), sent[0].answer.as_deref()), (None, None));
}

#[test]
fn a_row_that_names_no_channel_gets_the_one_that_answered() {
    let mut bare = row("j1", 10);
    bare.provenance = None;
    let rows = rows_of("beta", answer(vec![bare], Vec::new(), Some("next")));
    assert!(rows.more, "its own channel has a next page");
    assert_eq!(rows.entries[0].provenance.as_ref().and_then(|p| p.channel.as_deref()), Some("stable"));
}

#[test]
fn a_computer_with_no_rows_says_why() {
    let none = no_rows("beta", "it did not answer");
    assert_eq!((none.host.as_str(), none.entries.len(), none.note.as_deref()), ("beta", 0, Some("it did not answer")));
    assert!(note_for(Some("not_shared"), None).contains("sharing its Audit"));
    assert!(note_for(Some("needs_certificate"), None).contains("not yet approved"));
    assert!(note_for(Some("install_not_trusted"), None).contains("not yet approved"));
    assert!(note_for(Some("peer_cannot_do_remote_ops"), None).contains("cannot share"));
    assert_eq!(note_for(Some("stale_request"), None), "refused: stale_request");
    assert_eq!(note_for(None, Some("couldn't reach that machine")), "failed: couldn't reach that machine");
    assert_eq!(note_for(None, None), "it did not answer");
}
