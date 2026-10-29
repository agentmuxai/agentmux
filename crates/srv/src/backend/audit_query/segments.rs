// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent starts and ends, from the segment ledger (`continuity_segments`): one `agent_start` per segment and an
//! `agent_end` for each that ended, with the byte of the agent's history it began or ended at.

use super::{Ctx, KIND_AGENT_END, KIND_AGENT_START};
use crate::backend::continuity_segments::{self as ledger, EndReason, Segment};
use crate::backend::rpc_types::{AuditEntry, AuditHistoryLink, AuditLinks, AuditPaneLink};
use crate::backend::storage::filestore::FileStore;

/// Every agent that has a segment log, by UID.
fn agent_uids(store: &FileStore) -> Vec<String> {
    store
        .zone_ids_with_prefix("agent-uid:")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|zone| zone.strip_prefix("agent-uid:")?.strip_suffix(":segments").map(str::to_string))
        .collect()
}

fn reason_word(reason: EndReason) -> String {
    serde_json::to_value(reason).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The start and end entries of `segment`, for the agent known as `slug`.
fn entries_of(slug: &str, segment: &Segment, ctx: &Ctx) -> Vec<AuditEntry> {
    let start = &segment.start;
    let link = |at_ms: i64, byte: Option<i64>| AuditLinks {
        history: Some(AuditHistoryLink {
            agent_id: slug.to_string(),
            definition_id: start.definition_id.clone(),
            zone: start.zone.clone().filter(|_| byte.is_some()),
            at_ms: at_ms.max(0) as u64,
            at_byte: byte.map(|b| b.max(0) as u64),
        }),
        pane: Some(start.block_id.as_str()).filter(|b| !b.is_empty() && (ctx.block_exists)(b)).map(|b| AuditPaneLink { block_id: b.to_string() }),
    };
    let how = match start.continuity_rung {
        ledger::Rung::Native => "resumed its session",
        ledger::Rung::Virtualized => "started a new session with its record",
        ledger::Rung::Fresh => "started a new session",
    };
    let mut out = vec![AuditEntry {
        id: format!("s:{}:start", start.segment_id),
        ts_ms: start.started_at_ms.max(0) as u64,
        kind: KIND_AGENT_START.to_string(),
        agent_id: Some(slug.to_string()),
        from: None,
        to: None,
        summary: format!("{slug} {how} ({})", start.provider),
        outcome: None,
        reason: None,
        cost: None,
        links: link(start.started_at_ms, start.byte_start),
    }];
    if let (Some(at), Some(why)) = (segment.ended_at_ms, segment.end_reason) {
        let word = reason_word(why);
        out.push(AuditEntry {
            id: format!("s:{}:end", start.segment_id),
            ts_ms: at.max(0) as u64,
            kind: KIND_AGENT_END.to_string(),
            agent_id: Some(slug.to_string()),
            from: None,
            to: None,
            summary: format!("{slug} stopped ({word})"),
            outcome: None,
            reason: Some(word),
            cost: None,
            links: link(at, segment.byte_end),
        });
    }
    out
}

/// Every start and end entry of `store`'s agents (only `only`'s when given), unordered.
pub(super) fn entries(store: &FileStore, only: Option<&str>, ctx: &Ctx) -> Vec<AuditEntry> {
    let mut out = Vec::new();
    for uid in agent_uids(store) {
        let slug = (ctx.slug_of_uid)(&uid).unwrap_or_else(|| uid.clone());
        if let Some(want) = only {
            if !super::same_agent(&slug, want) {
                continue;
            }
        }
        for segment in ledger::segments(store, &uid) {
            out.extend(entries_of(&slug, &segment, ctx));
        }
    }
    out
}
