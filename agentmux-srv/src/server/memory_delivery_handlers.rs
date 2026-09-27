// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The endpoints Claude Code's `SessionStart` hook calls to deliver an agent's
//! memory (docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2). The
//! composition and the part rules are `backend::memory_delivery`.
//!
//! - `POST /api/v1/agent/memory/session-start/part` — one part. Each of the
//!   [`HOOK_PARTS`](memory_delivery::HOOK_PARTS) hook commands asks for its
//!   own part number. They run in parallel, so the first request for a
//!   (block, session, reason) composes the delivery and caches it, and every
//!   other part comes from that same composition.
//! - `POST /api/v1/agent/memory/session-start/ack` — a hook wrote its part to
//!   Claude. Once every part is acknowledged the delivery is complete: the pane
//!   gets one persisted `agentmux_memory_injected` frame naming each entry and
//!   its size (owner decision D11). A part fetched but never acknowledged (the
//!   hook crashed or timed out) leaves the delivery incomplete, and no notice
//!   claims otherwise.
//!
//! The agent comes from the request's credential (`X-Agent-Token` →
//! [`Caller::Agent`](super::caller::Caller)): its Personal Memory is that
//! agent's. Without a token only Global Memory is delivered.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::backend::memory_delivery::{self, Entry, Reason, Tier};

use super::caller::Caller;
use super::AppState;

/// How long a composed delivery stays cached for its remaining parts and
/// acknowledgements. The hooks of one session start run within seconds.
const DELIVERY_TTL_MS: i64 = 5 * 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DeliveryKey {
    block_id: String,
    session_id: String,
    reason: Reason,
}

#[derive(Debug, Clone)]
struct Delivery {
    parts: Vec<String>,
    entries: Vec<EntrySize>,
    summary_bytes: usize,
    acked: Vec<bool>,
    created_ms: i64,
    notice_sent: bool,
}

/// One entry of a delivery as the notice shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct EntrySize {
    pub label: String,
    pub source: &'static str,
    pub size_bytes: usize,
    pub tokens: usize,
}

static DELIVERIES: LazyLock<Mutex<HashMap<DeliveryKey, Delivery>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
pub(crate) struct PartRequest {
    block_id: String,
    session_id: String,
    /// The hook's `source` (`startup`, `resume`, `clear`, `compact`, `fork`).
    source: String,
    /// 1-based.
    part: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct PartResponse {
    /// This part's text, or `None` when there is none to deliver (no memory,
    /// a source that gets no delivery, or a part number past the last).
    text: Option<String>,
    part: usize,
    of: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AckRequest {
    block_id: String,
    session_id: String,
    source: String,
    part: usize,
}

/// `POST /api/v1/agent/memory/session-start/part`
pub(crate) async fn handle_session_start_part(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Json(req): Json<PartRequest>,
) -> impl IntoResponse {
    let Some(reason) = Reason::from_hook_source(&req.source) else {
        return Json(PartResponse { text: None, part: req.part, of: 0 });
    };
    let key = DeliveryKey { block_id: req.block_id.clone(), session_id: req.session_id.clone(), reason };
    let now = agentmux_common::time::now_ms();

    let cached = {
        let mut map = DELIVERIES.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, d| now - d.created_ms < DELIVERY_TTL_MS);
        map.get(&key).cloned()
    };
    let delivery = match cached {
        Some(d) => d,
        None => {
            let uid = caller.as_ref().and_then(|c| c.uid()).map(str::to_string);
            let composed = {
                let state = state.clone();
                let block_id = req.block_id.clone();
                tokio::task::spawn_blocking(move || compose_delivery(&state, &block_id, uid.as_deref(), reason, now))
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "memory delivery: composing failed");
                        None
                    })
            };
            let Some(composed) = composed else {
                return Json(PartResponse { text: None, part: req.part, of: 0 });
            };
            // A concurrent part may have composed first: keep that one, so
            // every part of this session start is from the same composition.
            let mut map = DELIVERIES.lock().unwrap_or_else(|e| e.into_inner());
            map.entry(key).or_insert(composed).clone()
        }
    };
    Json(part_response(&delivery, req.part))
}

/// `POST /api/v1/agent/memory/session-start/ack`
pub(crate) async fn handle_session_start_ack(State(state): State<AppState>, Json(req): Json<AckRequest>) -> impl IntoResponse {
    let Some(reason) = Reason::from_hook_source(&req.source) else {
        return Json(serde_json::json!({ "complete": false }));
    };
    let key = DeliveryKey { block_id: req.block_id.clone(), session_id: req.session_id.clone(), reason };
    let notice = {
        let mut map = DELIVERIES.lock().unwrap_or_else(|e| e.into_inner());
        map.get_mut(&key).and_then(|d| acknowledge(d, req.part).then(|| notice_frame(&key, d)))
    };
    let complete = notice.is_some();
    if let Some(frame) = notice {
        tracing::info!(block_id = %req.block_id, reason = reason.as_str(), "memory delivery: complete, notice sent");
        let line = format!("{frame}\n");
        let zone = crate::backend::blockcontroller::shell::resolve_global_output_zone(&Some(state.mstore.clone()), &req.block_id);
        crate::backend::blockcontroller::shell::handle_append_block_file(
            &state.broker,
            &req.block_id,
            crate::backend::blockcontroller::persistent::PERSISTENT_OUTPUT_SUBJECT,
            line.as_bytes(),
            Some(&state.filestore),
            zone.as_deref(),
        );
    }
    Json(serde_json::json!({ "complete": complete }))
}

/// Reads the memory and composes the delivery. `None` when there is nothing
/// to deliver.
fn compose_delivery(state: &AppState, block_id: &str, agent_uid: Option<&str>, reason: Reason, now: i64) -> Option<Delivery> {
    let mut entries = global_entries(state);
    if let Some(uid) = agent_uid {
        entries.extend(personal_entries(uid));
    }
    let summary = (reason == Reason::Compact)
        .then(|| crate::backend::continuity_state::running_summary_section(&state.mstore, block_id))
        .flatten();
    let text = memory_delivery::compose(&entries, reason, summary.as_deref())?;
    let parts = memory_delivery::split_into_parts(&text, memory_delivery::MAX_PART_CHARS, memory_delivery::HOOK_PARTS);
    let acked = vec![false; parts.len()];
    Some(Delivery {
        entries: entries.iter().map(entry_size).collect(),
        summary_bytes: summary.map_or(0, |s| s.len()),
        parts,
        acked,
        created_ms: now,
        notice_sent: false,
    })
}

/// Global Memory as the startup file carries it — its sections, Operator
/// Config first (`globalmemory:sections`, P1).
fn global_entries(state: &AppState) -> Vec<Entry> {
    let bundles = state.id_store.bundle_list_global().unwrap_or_default();
    crate::backend::storage::global_bundle_sections(&bundles)
        .into_iter()
        .map(|s| Entry {
            label: format!("{} {}", if s.is_system { "[AgentMux System]" } else { "[Workspace]" }, s.name),
            tier: Tier::Global,
            text: s.text,
        })
        .collect()
}

/// The agent's Personal Memory files from its latest spawn's memory folder,
/// by name, without the `MEMORY.md` index (the CLI's own table of contents).
fn personal_entries(agent_uid: &str) -> Vec<Entry> {
    let Some(dir) = crate::server::native_memory_handlers::memory_dir_from_spawn(agent_uid) else { return Vec::new() };
    personal_entries_in(&dir)
}

fn personal_entries_in(dir: &std::path::Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(String, std::path::PathBuf)> = read
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(name, _)| name.ends_with(".md") && name != "MEMORY.md")
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|(name, path)| {
            let text = std::fs::read_to_string(path).ok()?;
            (!text.trim().is_empty()).then_some(Entry { label: name, tier: Tier::Personal, text })
        })
        .collect()
}

fn entry_size(e: &Entry) -> EntrySize {
    EntrySize { label: e.label.clone(), source: e.tier.as_str(), size_bytes: e.size_bytes(), tokens: e.estimated_tokens() }
}

fn part_response(d: &Delivery, part: usize) -> PartResponse {
    let text = part.checked_sub(1).and_then(|i| d.parts.get(i)).cloned();
    PartResponse { text, part, of: d.parts.len() }
}

/// Marks `part` written. True exactly once: when this makes every part
/// written and no notice has gone out yet.
fn acknowledge(d: &mut Delivery, part: usize) -> bool {
    if let Some(slot) = part.checked_sub(1).and_then(|i| d.acked.get_mut(i)) {
        *slot = true;
    }
    if d.notice_sent || !d.acked.iter().all(|a| *a) {
        return false;
    }
    d.notice_sent = true;
    true
}

/// The persisted notice (D11): which memory was delivered, and each entry's
/// size. `id` is stable for the delivery, so a live frame and a replay of the
/// same line render once.
fn notice_frame(key: &DeliveryKey, d: &Delivery) -> serde_json::Value {
    let timestamp = chrono::DateTime::from_timestamp_millis(d.created_ms).map(|t| t.to_rfc3339()).unwrap_or_default();
    serde_json::json!({
        "type": "system",
        "subtype": "agentmux_memory_injected",
        "id": format!("memory-injected-{}-{}-{}", key.session_id, key.reason.as_str(), d.created_ms),
        "reason": key.reason.as_str(),
        "session_id": key.session_id,
        "parts": d.parts.len(),
        "entries": d.entries,
        "summary_bytes": d.summary_bytes,
        "timestamp": timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(parts: usize) -> Delivery {
        Delivery {
            parts: (1..=parts).map(|i| format!("part {i}")).collect(),
            entries: vec![EntrySize { label: "notes.md".into(), source: "personal", size_bytes: 5, tokens: 2 }],
            summary_bytes: 0,
            acked: vec![false; parts],
            created_ms: 1_790_000_000_000,
            notice_sent: false,
        }
    }

    #[test]
    fn parts_are_one_based_and_past_the_last_is_empty() {
        let d = delivery(2);
        assert_eq!(part_response(&d, 1), PartResponse { text: Some("part 1".into()), part: 1, of: 2 });
        assert_eq!(part_response(&d, 2).text.as_deref(), Some("part 2"));
        assert_eq!(part_response(&d, 3), PartResponse { text: None, part: 3, of: 2 });
        assert_eq!(part_response(&d, 0).text, None);
    }

    #[test]
    fn the_notice_goes_out_once_every_part_is_written_and_only_once() {
        let mut d = delivery(3);
        assert!(!acknowledge(&mut d, 1));
        assert!(!acknowledge(&mut d, 3));
        assert!(!acknowledge(&mut d, 3), "a repeated ack is not the missing part");
        assert!(acknowledge(&mut d, 2), "the last missing part completes it");
        assert!(!acknowledge(&mut d, 2), "never twice");
        assert!(!acknowledge(&mut d, 9), "an out-of-range part changes nothing");
    }

    #[test]
    fn an_unacknowledged_part_leaves_no_notice() {
        let mut d = delivery(2);
        assert!(!acknowledge(&mut d, 1));
        assert!(!d.notice_sent);
    }

    #[test]
    fn the_notice_names_each_entry_and_its_size() {
        let key = DeliveryKey { block_id: "b".into(), session_id: "s1".into(), reason: Reason::Startup };
        let f = notice_frame(&key, &delivery(1));
        assert_eq!(f["type"], "system");
        assert_eq!(f["subtype"], "agentmux_memory_injected");
        assert_eq!(f["reason"], "startup");
        assert_eq!(f["entries"][0]["label"], "notes.md");
        assert_eq!(f["entries"][0]["size_bytes"], 5);
        assert_eq!(f["entries"][0]["tokens"], 2);
        assert_eq!(f["id"], "memory-injected-s1-startup-1790000000000");
    }

    #[test]
    fn personal_entries_skip_the_index_empty_files_and_non_markdown() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("MEMORY.md"), "- [a](a.md)").unwrap();
        std::fs::write(dir.path().join("b.md"), "bee").unwrap();
        std::fs::write(dir.path().join("a.md"), "ay").unwrap();
        std::fs::write(dir.path().join("empty.md"), "  \n").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not memory").unwrap();
        let entries = personal_entries_in(dir.path());
        assert_eq!(
            entries.iter().map(|e| (e.label.as_str(), e.text.as_str())).collect::<Vec<_>>(),
            vec![("a.md", "ay"), ("b.md", "bee")]
        );
        assert!(entries.iter().all(|e| e.tier == Tier::Personal));
        assert!(personal_entries_in(&dir.path().join("missing")).is_empty());
    }
}
