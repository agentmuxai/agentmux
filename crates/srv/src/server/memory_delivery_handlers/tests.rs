// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of the memory-delivery endpoints: claims, compose gates, cards.

use super::*;

fn delivery(parts: usize) -> Delivery {
    Delivery {
        parts: (1..=parts).map(|i| format!("part {i}")).collect(),
        entries: vec![EntrySize {
            label: "notes.md".into(),
            source: "personal",
            size_bytes: 5,
            tokens: 2,
            kind: "personal_memory",
            name: "notes.md".into(),
            tier: None,
            bundle_id: None,
            path: None,
            delivered: "full",
            source_size_bytes: 5,
            source_tokens: 2,
            startup: None,
            via: None,
        }],
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
    assert_eq!(st.claim_fallback("b", Reason::Compact, NOW, i64::MIN), FallbackClaim::Deliver);
    assert!(st.fallback_claimed("b", Reason::Compact), "the hook's parts now come back empty");
    assert_eq!(st.claim_fallback("b", Reason::Compact, NOW, i64::MIN), FallbackClaim::Deliver, "a repeated claim is still the fallback's");
    assert!(!st.fallback_claimed("b", Reason::Startup), "another event is untouched");
    assert!(!st.fallback_claimed("other", Reason::Compact), "another block is untouched");
}

#[test]
fn a_complete_hook_delivery_makes_the_fallback_stand_down() {
    let mut st = state_with_hook_delivery("b", Reason::Compact, true);
    assert_eq!(st.claim_fallback("b", Reason::Compact, NOW, i64::MIN), FallbackClaim::Skip);
    assert!(!st.fallback_claimed("b", Reason::Compact));
}

#[test]
fn a_hook_delivery_in_flight_is_pending_until_the_fallback_takes_over() {
    let mut st = state_with_hook_delivery("b", Reason::Startup, false);
    assert_eq!(st.claim_fallback("b", Reason::Startup, NOW, i64::MIN), FallbackClaim::Pending);
    st.take_over_from_hook("b", Reason::Startup, NOW);
    assert!(st.fallback_claimed("b", Reason::Startup));
    let d = st.deliveries.values_mut().next().unwrap();
    assert!(!acknowledge(d, 1), "a late ack after the takeover adds no second notice");
}

fn startup_key(block: &str) -> DeliveryKey {
    DeliveryKey { block_id: block.into(), session_id: "s".into(), reason: Reason::Startup }
}

#[test]
fn a_fresh_session_claimed_after_a_long_first_turn_still_finds_its_hook_delivery() {
    // The hook delivered a second after the fresh-session frame; the
    // fallback waited out a 95 s first turn, past the claim window, and the
    // delivery itself has been pruned.
    let event_at = NOW;
    let mut st = DeliveryState::default();
    st.note_completed(&startup_key("b"), event_at + 1_000);
    let later = event_at + 95_000;
    st.prune(later);
    assert_eq!(st.claim_event("b", Reason::Startup, event_at, later), FallbackClaim::Skip);
    assert!(!st.fallback_claimed("b", Reason::Startup));
    // Undated, the same claim delivers a duplicate, as it used to.
    assert_eq!(st.claim_fallback("b", Reason::Startup, later, i64::MIN), FallbackClaim::Deliver);
}

#[test]
fn an_earlier_sessions_hook_delivery_does_not_cover_a_new_fresh_session() {
    let mut st = DeliveryState::default();
    st.note_completed(&startup_key("b"), NOW);
    let event_at = NOW + EVENT_SLACK_MS + 1;
    assert_eq!(st.claim_event("b", Reason::Startup, event_at, event_at + 1_000), FallbackClaim::Deliver);
    assert!(st.fallback_claimed("b", Reason::Startup), "the new session's hook now stands down");
}

#[test]
fn a_hook_delivery_just_before_its_sessions_frame_still_counts() {
    let mut st = DeliveryState::default();
    st.note_completed(&startup_key("b"), NOW);
    assert_eq!(st.claim_event("b", Reason::Startup, NOW + EVENT_SLACK_MS - 1, NOW + 120_000), FallbackClaim::Skip);
}

#[test]
fn a_dated_claim_waits_for_a_hook_delivery_in_flight() {
    let mut st = state_with_hook_delivery("b", Reason::Startup, false);
    let created = st.deliveries.values().next().unwrap().created_ms;
    assert_eq!(st.claim_event("b", Reason::Startup, created - 1_000, NOW), FallbackClaim::Pending);
}

#[test]
fn the_parts_of_one_delivery_share_a_compose_gate_and_a_finished_one_leaves_none() {
    let mut st = DeliveryState::default();
    let key = startup_key("b");
    let first = st.compose_gates.entry(key.clone()).or_default().clone();
    let second = st.compose_gates.entry(key.clone()).or_default().clone();
    assert!(std::sync::Arc::ptr_eq(&first, &second), "parts wait on the one composing");
    st.compose_gates.remove(&key);
    let next = st.compose_gates.entry(key).or_default().clone();
    assert!(!std::sync::Arc::ptr_eq(&first, &next), "a new delivery gets its own gate");
}

#[test]
fn a_gate_entry_takes_its_gate_with_it_when_dropped() {
    let key = startup_key("gate-entry-test");
    state_lock().compose_gates.entry(key.clone()).or_default();
    drop(GateEntry(key.clone()));
    assert!(!state_lock().compose_gates.contains_key(&key), "a cancelled part leaves no gate behind");
}

#[test]
fn a_dated_claim_is_per_block_and_reason() {
    let mut st = DeliveryState::default();
    st.note_completed(&startup_key("b"), NOW);
    assert_eq!(st.claim_event("other", Reason::Startup, NOW, NOW), FallbackClaim::Deliver);
    assert_eq!(st.claim_event("b", Reason::Compact, NOW, NOW), FallbackClaim::Deliver);
}

#[test]
fn completion_keeps_the_newest_delivery() {
    let mut st = DeliveryState::default();
    st.note_completed(&startup_key("b"), NOW);
    st.note_completed(&startup_key("b"), NOW - 5);
    assert_eq!(st.completed.get(&("b".to_string(), Reason::Startup)), Some(&NOW));
}

#[test]
fn stale_claims_and_deliveries_expire() {
    let mut st = state_with_hook_delivery("b", Reason::Compact, true);
    st.fallback_claims.insert(("b".into(), Reason::Startup), NOW);
    st.prune(NOW + CLAIM_WINDOW_MS + 1);
    assert!(!st.fallback_claimed("b", Reason::Startup));
    assert_eq!(
        st.claim_fallback("b", Reason::Compact, NOW + CLAIM_WINDOW_MS + 1, i64::MIN),
        FallbackClaim::Deliver,
        "a delivery older than the window is another event"
    );
}

#[test]
fn only_the_block_that_compacted_claims_its_boundary() {
    let mut st = DeliveryState::default();
    st.record_boundary("u1", "b", NOW);
    assert_eq!(
        st.claim_boundary("other", Reason::Compact, Some("u1"), NOW),
        FallbackClaim::Foreign,
        "another block of the agent, reading the shared zone"
    );
    assert_eq!(
        st.claim_boundary("b", Reason::Compact, Some("u2"), NOW),
        FallbackClaim::Foreign,
        "a boundary no stdout here wrote"
    );
    assert_eq!(st.claim_boundary("b", Reason::Compact, Some("u1"), NOW), FallbackClaim::Deliver);
    assert_eq!(
        st.claim_boundary("b", Reason::Compact, Some("u1"), NOW),
        FallbackClaim::Deliver,
        "a repeat by the same block delivers again: a duplicate over missing memory"
    );
}

#[test]
fn a_boundary_claim_still_stands_down_for_a_hook_delivery_in_the_window() {
    let mut st = state_with_hook_delivery("b", Reason::Compact, true);
    st.record_boundary("u1", "b", NOW);
    assert_eq!(st.claim_boundary("b", Reason::Compact, Some("u1"), NOW), FallbackClaim::Skip);
    let mut st = state_with_hook_delivery("b", Reason::Compact, true);
    st.record_boundary("u1", "b", NOW);
    assert_eq!(
        st.claim_boundary("b", Reason::Compact, Some("u1"), NOW + CLAIM_WINDOW_MS + 1),
        FallbackClaim::Deliver,
        "a hook delivery older than the window is another event"
    );
}

#[test]
fn an_earlier_compactions_hook_delivery_does_not_cover_the_next_boundary() {
    // The hook delivered compaction 1 (created NOW - 500) just before its
    // boundary; compaction 2 follows inside the claim window.
    let mut st = state_with_hook_delivery("b", Reason::Compact, true);
    st.record_boundary("u1", "b", NOW - 400);
    st.record_boundary("u2", "b", NOW - 100);
    assert_eq!(st.claim_boundary("b", Reason::Compact, Some("u1"), NOW), FallbackClaim::Skip);
    assert_eq!(st.claim_boundary("b", Reason::Compact, Some("u2"), NOW), FallbackClaim::Deliver);
}

fn compact_key(block: &str) -> DeliveryKey {
    DeliveryKey { block_id: block.into(), session_id: "s".into(), reason: Reason::Compact }
}

#[test]
fn a_boundary_claimed_long_after_the_hook_delivered_it_stands_down() {
    // The pane read the boundary late (a reconnect, a remounted pane
    // reading the gap): the hook's delivery is past the claim window and
    // pruned, but it finished after this boundary (SPEC_CONTEXT_DELIVERY
    // §9 B2).
    let mut st = DeliveryState::default();
    st.record_boundary("u1", "b", NOW);
    st.note_completed(&compact_key("b"), NOW + 300);
    let late = NOW + 10 * 60_000;
    st.prune(late);
    assert_eq!(st.claim_boundary("b", Reason::Compact, Some("u1"), late), FallbackClaim::Skip);
}

#[test]
fn a_late_boundary_claim_counts_a_delivery_composed_just_before_srv_read_the_boundary() {
    // The hook's first part can reach srv before the stdout reader sees
    // the boundary line.
    let mut st = DeliveryState::default();
    st.record_boundary("u1", "b", NOW);
    st.note_completed(&compact_key("b"), NOW - 2_000);
    assert_eq!(st.claim_boundary("b", Reason::Compact, Some("u1"), NOW + 10 * 60_000), FallbackClaim::Skip);
}

#[test]
fn an_earlier_compactions_finished_delivery_does_not_cover_a_late_claim() {
    let mut st = DeliveryState::default();
    st.record_boundary("u1", "b", NOW);
    st.note_completed(&compact_key("b"), NOW + 300);
    st.record_boundary("u2", "b", NOW + 3_600_000);
    assert_eq!(st.claim_boundary("b", Reason::Compact, Some("u2"), NOW + 3_700_000), FallbackClaim::Deliver);
}

#[test]
fn a_finished_delivery_is_per_block() {
    let mut st = DeliveryState::default();
    st.record_boundary("u1", "b", NOW);
    st.note_completed(&compact_key("other"), NOW + 300);
    assert_eq!(st.claim_boundary("b", Reason::Compact, Some("u1"), NOW + 10 * 60_000), FallbackClaim::Deliver);
}

#[test]
fn a_claim_without_a_boundary_keeps_the_old_behaviour() {
    let mut st = DeliveryState::default();
    assert_eq!(st.claim_boundary("b", Reason::Compact, None, NOW), FallbackClaim::Deliver);
}

#[test]
fn boundary_origins_are_bounded() {
    let mut st = DeliveryState::default();
    for i in 0..=BOUNDARY_ORIGINS_MAX {
        st.record_boundary(&format!("u{i}"), "b", NOW);
    }
    st.record_boundary("u5", "b", NOW);
    assert_eq!(st.boundary_origins.len(), BOUNDARY_ORIGINS_MAX);
    assert_eq!(st.boundary_origins.front().map(|(u, _, _)| u.as_str()), Some("u1"));
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

/// A new session's card lists the files the CLI loaded by itself, as
/// `startup_file` items with their role and owner flattened onto the item
/// (SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30 §4).
#[test]
fn startup_items_carry_role_owner_and_what_the_file_contains() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("ws");
    std::fs::create_dir_all(cwd.join(".claude")).unwrap();
    std::fs::write(cwd.join("CLAUDE.md"), "@.claude/AGENTMUX_MEMORY.md\n").unwrap();
    std::fs::write(cwd.join(".claude/AGENTMUX_MEMORY.md"), "# [AgentMux System] App API\n").unwrap();
    std::fs::write(cwd.join(".mcp.json"), r#"{"mcpServers":{"agentmux":{}}}"#).unwrap();
    let launch = LaunchDirs { cwd: cwd.clone(), config_dir: None };

    let items: Vec<serde_json::Value> =
        startup_items(&launch).iter().map(|i| serde_json::to_value(i).unwrap()).collect();
    let memory = items.iter().find(|i| i["role"] == "instructions_import").expect("the import");
    assert_eq!(memory["kind"], "startup_file");
    assert_eq!(memory["owner"], "agentmux");
    assert_eq!(memory["contains"], serde_json::json!(["global_memory"]));
    assert_eq!(memory["delivered"], "full");
    assert!(memory.get("startup").is_none(), "flattened, not nested");
    let own = items.iter().find(|i| i["role"] == "instructions").unwrap();
    assert_eq!(own["owner"], "user");
    assert!(own.get("contains").is_none(), "empty lists are left out");
    let mcp = items.iter().find(|i| i["role"] == "mcp_servers").unwrap();
    assert_eq!((mcp["count"].as_u64(), mcp["size_bytes"].as_u64()), (Some(1), Some(0)));
}

/// Only an AgentMux-written startup file holding every current section
/// counts as delivering the Global Memory (Codex on #4131).
#[test]
fn global_memory_counts_as_in_the_file_only_when_every_current_section_is() {
    use crate::backend::startup_files::{Owner, StartupFile};
    let entry = |name: &str, text: &str| Entry {
        label: name.into(),
        tier: Tier::Global,
        text: text.into(),
        name: name.into(),
        system: true,
        bundle_id: None,
        path: None,
    };
    let file = |owner: Owner, text: &str| StartupFile {
        role: "instructions_import",
        name: "AGENTMUX_MEMORY.md".into(),
        path: "/ws/.claude/AGENTMUX_MEMORY.md".into(),
        owner,
        text: text.into(),
        count: None,
        contains: vec!["global_memory"],
    };
    let global = [entry("A", "# [AgentMux System] A\n\nnew a"), entry("B", "# [Workspace] B\n\nb")];
    let both = "# Memory\n# [AgentMux System] A\n\nnew a\n\n---\n\n# [Workspace] B\n\nb\n";
    assert!(global_memory_in_startup_files(&[file(Owner::Agentmux, both)], &global));
    assert!(!global_memory_in_startup_files(&[file(Owner::User, both)], &global), "not AgentMux's");
    let stale = "# Memory\n# [AgentMux System] A\n\nold a\n\n---\n\n# [Workspace] B\n\nb\n";
    assert!(!global_memory_in_startup_files(&[file(Owner::Agentmux, stale)], &global), "an old section");
    assert!(!global_memory_in_startup_files(&[file(Owner::Agentmux, "# [Workspace] B\n\nb")], &global), "one missing");
    assert!(!global_memory_in_startup_files(&[file(Owner::Agentmux, both)], &[]), "nothing to deliver");
}

#[test]
fn launch_dirs_need_a_cwd_and_ignore_an_empty_config_dir() {
    let req = |cwd: &str, config_dir: &str| PartRequest {
        block_id: "b".into(),
        session_id: "s".into(),
        source: "startup".into(),
        part: 1,
        cwd: cwd.into(),
        config_dir: config_dir.into(),
    };
    assert!(LaunchDirs::from_request(&req("", "/cfg")).is_none(), "an older hook sends no cwd");
    let l = LaunchDirs::from_request(&req("/ws", " ")).unwrap();
    assert_eq!((l.cwd, l.config_dir), ("/ws".into(), None));
}

/// The fallback's card is the hook notice's shape under the delivery's
/// own id, flagged so replay knows the hidden echo after it (CD2b).
#[test]
fn a_fallback_card_names_its_delivery_and_reason() {
    let items = delivery(1).entries;
    let f = fallback_frame("fb-1", memory_delivery::FallbackReason::FreshSession, &items, 0, 1_790_000_000_000);
    assert_eq!(f["subtype"], "agentmux_memory_injected");
    assert_eq!(f["id"], "memory-injected-fb-1");
    assert_eq!(f["reason"], "resume_fresh");
    assert_eq!(f["fallback"], true);
    assert_eq!(f["delivery_id"], "fb-1");
    assert_eq!(f["entries"][0]["label"], "notes.md");
    let c = fallback_frame("fb-2", memory_delivery::FallbackReason::Compaction, &items, 0, 0);
    assert_eq!(c["reason"], "compact");
}

#[test]
fn a_fallback_card_is_taken_once_and_only_by_its_own_block() {
    let now = agentmux_common::time::now_ms();
    let put = |id: &str, block: &str| {
        state_lock().fallbacks.insert(
            id.to_string(),
            FallbackDelivery {
                delivery_id: id.to_string(),
                block_id: block.to_string(),
                text: String::new(),
                frame: serde_json::json!({"id": id}),
                created_ms: now,
            },
        );
    };
    put("fb-take-1", "blk-a");
    assert_eq!(take_fallback_frame("blk-a", "fb-take-1"), Some(serde_json::json!({"id": "fb-take-1"})));
    assert_eq!(take_fallback_frame("blk-a", "fb-take-1"), None, "once");
    put("fb-take-2", "blk-a");
    assert_eq!(take_fallback_frame("blk-b", "fb-take-2"), None, "another block's id");
    assert_eq!(take_fallback_frame("blk-a", "fb-unknown"), None);
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

fn item(label: &str, tier: Tier, text: &str) -> Entry {
    Entry {
        label: label.into(),
        tier,
        text: text.into(),
        name: label.into(),
        system: label.starts_with("sys"),
        bundle_id: (tier == Tier::Global).then(|| format!("id-{label}")),
        path: (tier == Tier::Personal).then(|| format!("/mem/{label}")),
    }
}

#[test]
fn items_carry_their_detail_and_follow_the_cut() {
    let entries = [
        item("sys-api", Tier::Global, &"a".repeat(100)),
        item("rules", Tier::Global, &"b".repeat(100)),
        item("notes.md", Tier::Personal, &"c".repeat(100)),
    ];
    let composed = memory_delivery::compose_items(&entries, Reason::Compact, Some(&"d".repeat(50))).unwrap();
    // Cut halfway through the second global entry.
    let cut = composed.spans[1].start + 40;
    let items = delivery_items(&entries, &composed, cut);

    assert_eq!(items.len(), 4);
    assert_eq!((items[0].kind, items[0].tier, items[0].delivered, items[0].size_bytes), ("global_memory", Some("system"), "full", 100));
    assert_eq!(items[0].bundle_id.as_deref(), Some("id-sys-api"));
    assert_eq!((items[1].tier, items[1].delivered, items[1].size_bytes), (Some("workspace"), "partial", 40));
    assert_eq!((items[2].kind, items[2].delivered, items[2].size_bytes), ("personal_memory", "omitted", 0));
    // The whole entry's size survives the cut, for the size band.
    assert_eq!((items[2].source_size_bytes, items[2].source_tokens), (100, 25));
    assert_eq!((items[1].source_size_bytes, items[1].size_bytes), (100, 40));
    assert_eq!(items[2].path.as_deref(), Some("/mem/notes.md"));
    assert_eq!((items[3].kind, items[3].source, items[3].delivered), ("running_summary", "summary", "omitted"));
}

#[test]
fn an_uncut_delivery_reports_every_item_whole() {
    let entries = [item("rules", Tier::Global, "rules text"), item("n.md", Tier::Personal, "notes")];
    let composed = memory_delivery::compose_items(&entries, Reason::Startup, None).unwrap();
    let items = delivery_items(&entries, &composed, composed.text.chars().count());
    assert!(items.iter().all(|i| i.delivered == "full"));
    assert_eq!(items.iter().map(|i| i.size_bytes).collect::<Vec<_>>(), vec![10, 5]);
    let json = serde_json::to_value(&items[1]).unwrap();
    assert!(json.get("tier").is_none() && json.get("bundle_id").is_none(), "{json}");
    assert_eq!(json["path"], "/mem/n.md");
}
