// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The paired computers on the LAN, asked for their Audit rows (SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md
//! §3.6 point 5). A computer's answer is a page of its own `audit.query` on the `machine` scope; here it becomes the rows
//! of one section: each row says which computer it is from, has an id of its own, and keeps no link that only works on
//! the computer that wrote it (a pane, an agent's history, a Kanban task). The rows are shown and forgotten: nothing
//! here stores them, and the cloud never sees them.

use crate::backend::rpc_types::{AuditEntry, AuditLinks, AuditPeerRows, AuditQueryReq, AuditQueryResult};

/// The remote operation the Audit asks a paired computer with.
pub const QUERY_OP: &str = "audit.query";

/// The setting that lets paired computers read this computer's Audit. Off unless set.
pub const SETTING_AUDIT_SHARE: &str = "audit:sharewithpaired";

/// The most rows one paired computer is asked for, and answers with.
pub const PEER_LIMIT: u32 = 100;

/// What a paired computer is asked, and what it answers: the filter of `req`, for the first page of every channel on
/// that computer. Its paging, scope and totals are the asker's own.
pub fn request_for_peer(req: &AuditQueryReq) -> AuditQueryReq {
    AuditQueryReq { cursor: None, totals: None, scope: Some("machine".to_string()), limit: Some(req.limit.unwrap_or(50).min(PEER_LIMIT)), ..req.clone() }
}

/// Take from `entries` the links that only work on the computer that wrote them: a pane, an agent's history and a task
/// of its Kanban would open nothing, or the wrong thing, anywhere else.
pub fn without_local_links(entries: &mut [AuditEntry]) {
    for e in entries {
        e.links = AuditLinks::default();
        for end in [e.from.as_mut(), e.to.as_mut()].into_iter().flatten() {
            end.links = AuditLinks::default();
        }
    }
}

/// `host`'s answer as the rows of its section: its own channel's rows and its other channels' in one list, newest first.
pub fn rows_of(host: &str, result: AuditQueryResult) -> AuditPeerRows {
    let more = result.next_cursor.is_some() || result.other_channels.iter().flatten().any(|c| c.more);
    let mut entries = result.entries;
    for channel in result.other_channels.unwrap_or_default() {
        entries.extend(channel.entries);
    }
    entries.sort_by(|a, b| (b.ts_ms, &b.id).cmp(&(a.ts_ms, &a.id)));
    without_local_links(&mut entries);
    for e in &mut entries {
        e.id = format!("{host}/{}", e.id);
        let provenance = e.provenance.get_or_insert_with(Default::default);
        // The computer is the one asked, named as `Agents list` names it, so the rows of one computer are one section.
        provenance.host = Some(host.to_string());
        if provenance.channel.as_deref().is_none_or(str::is_empty) {
            provenance.channel = result.channel.clone();
        }
    }
    AuditPeerRows { host: host.to_string(), entries, more, note: None }
}

/// A computer that gave no rows, and why.
pub fn no_rows(host: &str, note: &str) -> AuditPeerRows {
    AuditPeerRows { host: host.to_string(), entries: Vec::new(), more: false, note: Some(note.to_string()) }
}

/// What a person is told when a paired computer's answer is a refusal or a failure: the reason, in words.
pub fn note_for(refused_reason: Option<&str>, error: Option<&str>) -> String {
    match refused_reason {
        Some("not_shared") => "its owner has not turned on sharing its Audit with paired computers".to_string(),
        Some("needs_certificate") | Some("install_not_trusted") => "this computer is not yet approved there".to_string(),
        Some("remote_ops_off") => "it does not take requests from other computers".to_string(),
        Some("peer_cannot_do_remote_ops") | Some("unknown_op") => "it runs a version that cannot share its Audit".to_string(),
        Some(other) => format!("refused: {other}"),
        None => error.map_or_else(|| "it did not answer".to_string(), |e| format!("failed: {e}")),
    }
}

#[cfg(test)]
#[path = "peers_tests.rs"]
mod tests;
