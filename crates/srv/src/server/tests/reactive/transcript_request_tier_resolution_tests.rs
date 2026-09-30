// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `transcript_request_tier_resolution_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;
use crate::backend::storage::store::AgentDefinition;
use crate::server::tests::test_state;

fn insert_agent_def(state: &AppState, slug: &str, conversation_visibility: &str) {
    let mut def = AgentDefinition {
        id: uuid::Uuid::new_v4().to_string(),
        slug: slug.to_string(),
        name: slug.to_string(),
        icon: String::new(),
        provider: "claude".to_string(),
        description: String::new(),
        working_directory: String::new(),
        shell: String::new(),
        provider_flags: String::new(),
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: "standalone".to_string(),
        environment: String::new(),
        agent_bus_id: String::new(),
        is_seeded: 0,
        accounts: String::new(),
        parent_id: String::new(),
        branch_label: String::new(),
        updated_at: 0,
        user_hidden: 0,
        container_image: String::new(),
        container_volumes: "[]".to_string(),
        container_name: String::new(),
        use_ambient_login: 0,
        model_vendor_base_url: String::new(),
        auto_continue_enabled: 0,
        memory_id: String::new(),
        conversation_visibility: conversation_visibility.to_string(),
    };
    state.mstore.agent_def_insert(&mut def).unwrap();
}

fn transcript_request_message() -> String {
    r#"{"type":"transcript_request","request_id":"r1","max_lines":50}"#.to_string()
}

fn base_req(target: &str) -> InjectionRequest {
    InjectionRequest {
        target_agent: target.to_string(),
        message: transcript_request_message(),
        source_agent: Some("requester".to_string()),
        delivery_tier: Some("lan".to_string()),
        ..Default::default()
    }
}

#[tokio::test]
async fn ordinary_message_never_sets_either_field() {
    let state = test_state();
    let mut req = InjectionRequest {
        target_agent: "agent1".to_string(),
        message: "just chatting".to_string(),
        ..Default::default()
    };
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);
    assert!(!req.is_transcript_request);
    assert!(!req.transcript_request_escalate_forced);
}

#[tokio::test]
async fn private_visibility_does_not_force_escalate() {
    let state = test_state();
    insert_agent_def(&state, "agent1", "private");
    let mut req = base_req("agent1");
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);
    assert!(req.is_transcript_request);
    assert!(!req.transcript_request_escalate_forced);
}

#[tokio::test]
async fn ask_visibility_forces_escalate() {
    let state = test_state();
    insert_agent_def(&state, "agent1", "ask");
    let mut req = base_req("agent1");
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);
    assert!(req.is_transcript_request);
    assert!(req.transcript_request_escalate_forced);
}

#[tokio::test]
async fn trusted_peers_without_a_grant_forces_escalate() {
    let state = test_state();
    insert_agent_def(&state, "agent1", "trusted_peers");
    let mut req = base_req("agent1");
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);
    assert!(
        req.transcript_request_escalate_forced,
        "an un-granted requester must still force escalation under trusted_peers mode"
    );
}

#[tokio::test]
async fn trusted_peers_with_a_matching_grant_does_not_force_escalate() {
    let state = test_state();
    insert_agent_def(&state, "agent1", "trusted_peers");
    state.mstore.conversation_trust_grant_add("agent1", "requester", "lan").unwrap();
    let mut req = base_req("agent1");
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);
    assert!(
        !req.transcript_request_escalate_forced,
        "an allow-listed requester on the SAME tier must not force escalation"
    );
}

#[tokio::test]
async fn trusted_peers_grant_on_a_different_tier_still_forces_escalate() {
    let state = test_state();
    insert_agent_def(&state, "agent1", "trusted_peers");
    // Granted for WAN, but this request arrives on LAN (base_req's default).
    state.mstore.conversation_trust_grant_add("agent1", "requester", "wan").unwrap();
    let mut req = base_req("agent1");
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);
    assert!(
        req.transcript_request_escalate_forced,
        "a grant for one tier's identity guarantee must never be assumed to cover a different tier"
    );
}

// Phase C (WAN): this function is tier-generic — these two tests pin
// that WAN delivery gets the identical treatment LAN already has,
// since `sync_agent_reactive` (the WAN delivery path,
// `muxbus/cloud_subscriber.rs`) calls this exact function directly.
#[tokio::test]
async fn wan_tier_transcript_request_forces_sensitive_same_as_lan() {
    let state = test_state();
    insert_agent_def(&state, "agent1", "private");
    let mut req = InjectionRequest {
        target_agent: "agent1".to_string(),
        message: transcript_request_message(),
        source_agent: Some("requester".to_string()),
        delivery_tier: Some("wan".to_string()),
        ..Default::default()
    };
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);
    assert!(req.is_transcript_request, "rule 1 must fire on WAN exactly like every other tier");
}

/// W3-S (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.6) reverses what
/// this test used to pin: a `wan` grant no longer relaxes escalation. It
/// is keyed by bare peer name, and on WAN one name can be several
/// instances (anyone holding the account token can mint one), so honouring
/// it waits for an instance-keyed grant. No production path creates
/// grants yet, so nothing that worked stops working.
#[tokio::test]
async fn wan_tier_trusted_peers_grant_is_not_honoured_yet() {
    let state = test_state();
    insert_agent_def(&state, "agent1", "trusted_peers");
    state.mstore.conversation_trust_grant_add("agent1", "requester", "wan").unwrap();
    let mut req = InjectionRequest {
        target_agent: "agent1".to_string(),
        message: transcript_request_message(),
        source_agent: Some("requester".to_string()),
        delivery_tier: Some("wan".to_string()),
        ..Default::default()
    };
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);
    assert!(
        req.transcript_request_escalate_forced,
        "a WAN grant is keyed by name only, and one name can be several instances on WAN"
    );
}

#[tokio::test]
async fn matches_on_slug_not_display_name() {
    let state = test_state();
    // Insert with a slug matching the target, but a DIFFERENT display name —
    // the lookup must key off slug (the stable AGENTMUX_AGENT_ID-derived
    // identifier), same cross-namespace hazard the Supervisor-nudge
    // opt-in check just above this function already guards against.
    let state_clone = &state;
    {
        let mut def = AgentDefinition {
            id: uuid::Uuid::new_v4().to_string(),
            slug: "agent1".to_string(),
            name: "Totally Different Display Name".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            model_vendor_base_url: String::new(),
            auto_continue_enabled: 0,
            memory_id: String::new(),
            conversation_visibility: "ask".to_string(),
        };
        state_clone.mstore.agent_def_insert(&mut def).unwrap();
    }
    let mut req = base_req("agent1");
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);
    assert!(req.transcript_request_escalate_forced, "lookup must match by slug \"agent1\", not the unrelated display name");
}

#[tokio::test]
async fn unknown_target_agent_fails_closed_to_private_defaults() {
    let state = test_state();
    // No AgentDefinition inserted at all for this target.
    let mut req = base_req("no-such-agent");
    resolve_transcript_request_tier_fields(&state.mstore, &mut req);
    assert!(req.is_transcript_request, "rule 1 (forced sensitive) applies regardless of whether the target is known");
    assert!(!req.transcript_request_escalate_forced, "an unknown agent defaults to the safe 'private' behavior for the escalate-forcing question");
}
