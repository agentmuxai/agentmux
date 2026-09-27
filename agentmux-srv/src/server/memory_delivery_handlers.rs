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

/// How long a claim on one event (a session start, a compaction) holds: the
/// hook and the frontend's fallback for the same event arrive within
/// seconds of each other, and one block has no two such events that close.
const CLAIM_WINDOW_MS: i64 = 60_000;

/// How long the fallback waits for a hook delivery still in flight, and how
/// often it looks. A hook that never finishes must not leave the agent with
/// no memory, so after this the fallback delivers instead.
const PENDING_WAIT: std::time::Duration = std::time::Duration::from_secs(4);
const PENDING_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// Deliveries and fallback claims, behind one lock so "who claimed this event
/// first" is decided atomically.
#[derive(Default)]
struct DeliveryState {
    deliveries: HashMap<DeliveryKey, Delivery>,
    /// Events the frontend's hidden-reinjection fallback claimed:
    /// (block, reason) → when.
    fallback_claims: HashMap<(String, Reason), i64>,
}

/// What the fallback's claim found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FallbackClaim {
    /// Nothing delivered this event yet, or the fallback claimed it already:
    /// the fallback delivers.
    Deliver,
    /// The hook delivered this event completely: the fallback stands down.
    Skip,
    /// The hook is delivering this event right now.
    Pending,
}

impl DeliveryState {
    fn prune(&mut self, now: i64) {
        self.deliveries.retain(|_, d| now - d.created_ms < DELIVERY_TTL_MS);
        self.fallback_claims.retain(|_, at| now - *at < CLAIM_WINDOW_MS);
    }

    fn fallback_claimed(&self, block_id: &str, reason: Reason) -> bool {
        self.fallback_claims.contains_key(&(block_id.to_string(), reason))
    }

    /// The hook's deliveries of this event, newest first.
    fn hook_deliveries_mut(&mut self, block_id: &str, reason: Reason, now: i64) -> Vec<&mut Delivery> {
        let mut found: Vec<&mut Delivery> = self
            .deliveries
            .iter_mut()
            .filter(|(k, d)| k.block_id == block_id && k.reason == reason && now - d.created_ms < CLAIM_WINDOW_MS)
            .map(|(_, d)| d)
            .collect();
        found.sort_by_key(|d| std::cmp::Reverse(d.created_ms));
        found
    }

    /// The fallback's atomic claim on an event. A `Deliver` for an unclaimed
    /// event records the claim, so the hook's parts for it come back empty.
    fn claim_fallback(&mut self, block_id: &str, reason: Reason, now: i64) -> FallbackClaim {
        if self.fallback_claimed(block_id, reason) {
            return FallbackClaim::Deliver;
        }
        let hook = self.hook_deliveries_mut(block_id, reason, now).into_iter().next().map(|d| d.notice_sent);
        match hook {
            Some(true) => FallbackClaim::Skip,
            Some(false) => FallbackClaim::Pending,
            None => {
                self.fallback_claims.insert((block_id.to_string(), reason), now);
                FallbackClaim::Deliver
            }
        }
    }

    /// The fallback gave up waiting for the hook: it delivers, and the hook's
    /// unfinished delivery is closed so a late acknowledgement adds no second
    /// notice.
    fn take_over_from_hook(&mut self, block_id: &str, reason: Reason, now: i64) {
        for d in self.hook_deliveries_mut(block_id, reason, now) {
            d.notice_sent = true;
        }
        self.fallback_claims.insert((block_id.to_string(), reason), now);
    }
}

static STATE: LazyLock<Mutex<DeliveryState>> = LazyLock::new(|| Mutex::new(DeliveryState::default()));

fn state_lock() -> std::sync::MutexGuard<'static, DeliveryState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

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
        let mut st = state_lock();
        st.prune(now);
        // The frontend's fallback already delivered this event: stand down.
        if st.fallback_claimed(&req.block_id, reason) {
            return Json(PartResponse { text: None, part: req.part, of: 0 });
        }
        st.deliveries.get(&key).cloned()
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
            // And the fallback may have claimed the event meanwhile.
            let mut st = state_lock();
            if st.fallback_claimed(&req.block_id, reason) {
                return Json(PartResponse { text: None, part: req.part, of: 0 });
            }
            st.deliveries.entry(key).or_insert(composed).clone()
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
        let mut st = state_lock();
        st.deliveries.get_mut(&key).and_then(|d| acknowledge(d, req.part).then(|| notice_frame(&key, d)))
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

/// The event a frontend reinjection reason stands for: a compaction, or a
/// fresh session (a spawn without `--resume`, whose hook `source` is
/// `startup`). `None` for anything else.
fn reason_for_fallback(reason: &str) -> Option<Reason> {
    match reason {
        "compaction" => Some(Reason::Compact),
        "fresh_session" => Some(Reason::Startup),
        _ => None,
    }
}

/// Whether the frontend's hidden-reinjection fallback should deliver this
/// event (`true`) or stand down because the `SessionStart` hook delivered it
/// (`false`). Waits out a hook delivery still in flight for up to
/// [`PENDING_WAIT`], then lets the fallback deliver, so a hook that died
/// part-way never leaves the agent without its memory.
pub(crate) async fn claim_fallback(block_id: &str, reason: &str) -> bool {
    let Some(reason) = reason_for_fallback(reason) else { return true };
    let deadline = tokio::time::Instant::now() + PENDING_WAIT;
    loop {
        let now = agentmux_common::time::now_ms();
        let claim = {
            let mut st = state_lock();
            st.prune(now);
            st.claim_fallback(block_id, reason, now)
        };
        match claim {
            FallbackClaim::Deliver => return true,
            FallbackClaim::Skip => {
                tracing::info!(block_id, reason = reason.as_str(), "memory delivery: the hook delivered; fallback stands down");
                return false;
            }
            FallbackClaim::Pending if tokio::time::Instant::now() >= deadline => {
                tracing::warn!(block_id, reason = reason.as_str(), "memory delivery: hook still unfinished; fallback delivers instead");
                state_lock().take_over_from_hook(block_id, reason, agentmux_common::time::now_ms());
                return true;
            }
            FallbackClaim::Pending => tokio::time::sleep(PENDING_POLL).await,
        }
    }
}

/// Registers `memorydelivery:claim_fallback` — the frontend's hidden memory
/// reinjection asks it right before it would fire.
pub(crate) fn register_memory_delivery_handlers(engine: &std::sync::Arc<crate::backend::rpc::engine::WshRpcEngine>) {
    use crate::backend::rpc_types::{
        CommandMemoryDeliveryClaimFallbackData, MemoryDeliveryClaimFallbackResult, COMMAND_MEMORY_DELIVERY_CLAIM_FALLBACK,
    };
    engine.register_typed(
        COMMAND_MEMORY_DELIVERY_CLAIM_FALLBACK,
        |req: CommandMemoryDeliveryClaimFallbackData, _ctx| async move {
            Ok::<_, String>(MemoryDeliveryClaimFallbackResult { deliver: claim_fallback(&req.block_id, &req.reason).await })
        },
    );
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

    fn state_with_hook_delivery(block: &str, reason: Reason, notice_sent: bool) -> DeliveryState {
        let mut st = DeliveryState::default();
        let mut d = delivery(1);
        d.notice_sent = notice_sent;
        st.deliveries.insert(DeliveryKey { block_id: block.into(), session_id: "s".into(), reason }, d);
        st
    }
    const NOW: i64 = 1_790_000_000_500;

    #[test]
    fn an_unclaimed_event_goes_to_the_fallback_and_the_hook_then_stands_down() {
        let mut st = DeliveryState::default();
        assert_eq!(st.claim_fallback("b", Reason::Compact, NOW), FallbackClaim::Deliver);
        assert!(st.fallback_claimed("b", Reason::Compact), "the hook's parts now come back empty");
        assert_eq!(st.claim_fallback("b", Reason::Compact, NOW), FallbackClaim::Deliver, "a repeated claim is still the fallback's");
        assert!(!st.fallback_claimed("b", Reason::Startup), "another event is untouched");
        assert!(!st.fallback_claimed("other", Reason::Compact), "another block is untouched");
    }

    #[test]
    fn a_complete_hook_delivery_makes_the_fallback_stand_down() {
        let mut st = state_with_hook_delivery("b", Reason::Compact, true);
        assert_eq!(st.claim_fallback("b", Reason::Compact, NOW), FallbackClaim::Skip);
        assert!(!st.fallback_claimed("b", Reason::Compact));
    }

    #[test]
    fn a_hook_delivery_in_flight_is_pending_until_the_fallback_takes_over() {
        let mut st = state_with_hook_delivery("b", Reason::Startup, false);
        assert_eq!(st.claim_fallback("b", Reason::Startup, NOW), FallbackClaim::Pending);
        st.take_over_from_hook("b", Reason::Startup, NOW);
        assert!(st.fallback_claimed("b", Reason::Startup));
        let d = st.deliveries.values_mut().next().unwrap();
        assert!(!acknowledge(d, 1), "a late ack after the takeover adds no second notice");
    }

    #[test]
    fn stale_claims_and_deliveries_expire() {
        let mut st = state_with_hook_delivery("b", Reason::Compact, true);
        st.fallback_claims.insert(("b".into(), Reason::Startup), NOW);
        st.prune(NOW + CLAIM_WINDOW_MS + 1);
        assert!(!st.fallback_claimed("b", Reason::Startup));
        assert_eq!(
            st.claim_fallback("b", Reason::Compact, NOW + CLAIM_WINDOW_MS + 1),
            FallbackClaim::Deliver,
            "a delivery older than the window is another event"
        );
    }

    #[test]
    fn the_fallback_reasons_map_to_hook_events() {
        assert_eq!(reason_for_fallback("compaction"), Some(Reason::Compact));
        assert_eq!(reason_for_fallback("fresh_session"), Some(Reason::Startup));
        assert_eq!(reason_for_fallback("other"), None);
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
