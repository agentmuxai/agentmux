// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, Mutex};

use super::handler::*;
use super::poller::*;
use super::sanitize::*;
use super::types::*;
use super::*;

// -- Sanitization tests --

#[test]
fn test_sanitize_plain_text() {
    assert_eq!(sanitize_message("hello world"), "hello world");
}

#[test]
fn test_sanitize_preserves_whitespace() {
    assert_eq!(sanitize_message("line1\nline2\ttab"), "line1\nline2\ttab");
}

#[test]
fn test_sanitize_removes_ansi_escape() {
    assert_eq!(sanitize_message("hello\x1b[31mred\x1b[0m"), "hellored");
}

#[test]
fn test_sanitize_removes_osc_sequence() {
    assert_eq!(
        sanitize_message("before\x1b]0;title\x07after"),
        "beforeafter"
    );
}

#[test]
fn test_sanitize_removes_osc_with_st() {
    assert_eq!(
        sanitize_message("before\x1b]0;title\x1b\\after"),
        "beforeafter"
    );
}

#[test]
fn test_sanitize_removes_control_chars() {
    assert_eq!(sanitize_message("hello\x01\x02world"), "helloworld");
}

#[test]
fn test_sanitize_removes_del() {
    assert_eq!(sanitize_message("hello\x7fworld"), "helloworld");
}

#[test]
fn test_sanitize_truncates_long_message() {
    let long_msg = "x".repeat(MAX_MESSAGE_LENGTH + 100);
    let result = sanitize_message(&long_msg);
    assert!(result.len() <= MAX_MESSAGE_LENGTH);
    assert!(result.ends_with(TRUNCATION_SUFFIX));
}

#[test]
fn test_sanitize_preserves_unicode() {
    assert_eq!(sanitize_message("hello 世界 🌍"), "hello 世界 🌍");
}

#[test]
fn test_sanitize_empty() {
    assert_eq!(sanitize_message(""), "");
}

// -- Agent ID validation tests --

#[test]
fn test_validate_agent_id_valid() {
    assert!(validate_agent_id("Agent1"));
    assert!(validate_agent_id("my_agent-2"));
    assert!(validate_agent_id("a"));
}

#[test]
fn test_validate_agent_id_invalid() {
    assert!(!validate_agent_id(""));
    assert!(!validate_agent_id("agent with spaces"));
    assert!(!validate_agent_id("agent@special"));
    let long_id = "a".repeat(65);
    assert!(!validate_agent_id(&long_id));
}

#[test]
fn test_validate_agent_id_max_length() {
    let id = "a".repeat(64);
    assert!(validate_agent_id(&id));
}

// -- URL validation tests --

#[test]
fn test_validate_url_https() {
    assert!(validate_muxbus_url("https://agentmux.example.com/api").is_ok());
}

#[test]
fn test_validate_url_http_localhost() {
    assert!(validate_muxbus_url("http://localhost:8080/api").is_ok());
    assert!(validate_muxbus_url("http://127.0.0.1:8080/api").is_ok());
    assert!(validate_muxbus_url("http://[::1]:8080/api").is_ok());
}

#[test]
fn test_validate_url_http_remote_rejected() {
    assert!(validate_muxbus_url("http://evil.com/api").is_err());
}

#[test]
fn test_validate_url_bad_scheme() {
    assert!(validate_muxbus_url("ftp://example.com").is_err());
    assert!(validate_muxbus_url("file:///etc/passwd").is_err());
}

#[test]
fn test_validate_url_empty() {
    assert!(validate_muxbus_url("").is_err());
}

#[test]
fn test_validate_url_no_scheme() {
    assert!(validate_muxbus_url("example.com/api").is_err());
}

// -- Format injected message tests --

#[test]
fn test_format_with_source() {
    assert_eq!(
        format_injected_message("hello", Some("Agent1"), true),
        "@Agent1: hello"
    );
}

#[test]
fn test_format_without_source() {
    assert_eq!(
        format_injected_message("hello", Some("Agent1"), false),
        "hello"
    );
}

#[test]
fn test_format_no_source_agent() {
    assert_eq!(format_injected_message("hello", None, true), "hello");
}

// -- Rate limiter tests --

#[test]
fn test_rate_limiter_allows_within_limit() {
    let mut rl = super::handler::RateLimiter::new(3);
    assert!(rl.check());
    assert!(rl.check());
    assert!(rl.check());
}

#[test]
fn test_rate_limiter_blocks_over_limit() {
    let mut rl = super::handler::RateLimiter::new(2);
    assert!(rl.check());
    assert!(rl.check());
    assert!(!rl.check());
}

// -- Handler tests --

#[test]
fn test_handler_register_and_get() {
    let mut handler = Handler::new();
    handler
        .register_agent("agent1", "block1", Some("tab1"))
        .unwrap();

    let agent = handler.get_agent("agent1").unwrap();
    assert_eq!(agent.block_id, "block1");
    assert_eq!(agent.tab_id.as_deref(), Some("tab1"));
}

#[test]
fn test_handler_register_replaces_existing() {
    let mut handler = Handler::new();
    handler
        .register_agent("agent1", "block1", None)
        .unwrap();
    handler
        .register_agent("agent1", "block2", None)
        .unwrap();

    let agent = handler.get_agent("agent1").unwrap();
    assert_eq!(agent.block_id, "block2");
    assert!(handler.get_agent_by_block("block1").is_none());
}

#[test]
fn test_handler_unregister_agent() {
    let mut handler = Handler::new();
    handler
        .register_agent("agent1", "block1", None)
        .unwrap();
    handler.unregister_agent("agent1");

    assert!(handler.get_agent("agent1").is_none());
    assert!(handler.get_agent_by_block("block1").is_none());
}

#[test]
fn test_handler_unregister_block() {
    let mut handler = Handler::new();
    handler
        .register_agent("agent1", "block1", None)
        .unwrap();
    handler.unregister_block("block1");

    assert!(handler.get_agent("agent1").is_none());
}

/// Issue #2363 / codex P1 on PR #2500: the guarded unregister removes
/// only its own spawn's registration — a newer spawn's (or replacement
/// controller's) re-registration under a different nonce must survive a
/// stale exit-handler's cleanup.
#[test]
fn test_handler_unregister_block_if_nonce_spares_a_newer_registration() {
    let mut handler = Handler::new();
    handler
        .register_agent_with_nonce("agent1", "block1", None, 5)
        .unwrap();
    // A fallback respawn (or a resync_controller replacement) re-registers
    // the same agent/block under its own nonce before the dying spawn's
    // exit-handler reaches cleanup.
    handler
        .register_agent_with_nonce("agent1", "block1", None, 6)
        .unwrap();

    assert!(
        !handler.unregister_block_if_nonce("block1", 5),
        "the stale spawn's cleanup must not claim the newer registration"
    );
    assert!(
        handler.get_agent("agent1").is_some(),
        "the newer registration must survive"
    );

    assert!(handler.unregister_block_if_nonce("block1", 6), "the owner may remove it");
    assert!(handler.get_agent("agent1").is_none());
}

/// A registration with no recorded nonce (HTTP/PTY register paths pass 0)
/// is never removed by the guarded variant — stale-entry leakage is
/// strictly safer than deleting a live registration.
#[test]
fn test_handler_unregister_block_if_nonce_never_matches_nonceless_registrations() {
    let mut handler = Handler::new();
    handler.register_agent("agent1", "block1", None).unwrap();

    assert!(!handler.unregister_block_if_nonce("block1", 0));
    assert!(handler.get_agent("agent1").is_some());
}

#[test]
fn test_handler_list_agents() {
    let mut handler = Handler::new();
    handler
        .register_agent("agent1", "block1", None)
        .unwrap();
    handler
        .register_agent("agent2", "block2", None)
        .unwrap();

    let agents = handler.list_agents();
    assert_eq!(agents.len(), 2);
}

#[test]
fn test_handler_invalid_agent_id() {
    let mut handler = Handler::new();
    let result = handler.register_agent("invalid agent!", "block1", None);
    assert!(result.is_err());
}

#[test]
fn test_handler_inject_no_sender() {
    let mut handler = Handler::new();
    handler
        .register_agent("agent1", "block1", None)
        .unwrap();

    let resp = handler.inject_message(InjectionRequest {
        target_agent: "agent1".to_string(),
        message: "hello".to_string(),
        source_agent: None,
        request_id: None,
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: None,
        forward_hops: 0,
    });

    assert!(!resp.success);
    assert!(resp.error.unwrap().contains("input sender not configured"));
}

#[test]
fn test_handler_inject_agent_not_found() {
    let mut handler = Handler::new();

    let resp = handler.inject_message(InjectionRequest {
        target_agent: "nonexistent".to_string(),
        message: "hello".to_string(),
        source_agent: None,
        request_id: None,
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: None,
        forward_hops: 0,
    });

    assert!(!resp.success);
    assert!(resp.error.unwrap().contains("agent not found"));
}

// `#[tokio::test]` because `Handler::inject_message` internally
// `tokio::spawn`s the delayed-Enter follow-up; without a runtime it
// panics at the spawn site (handler.rs:308).
#[tokio::test]
async fn test_handler_inject_success() {
    let sent = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
    let sent_clone = sent.clone();

    let mut handler = Handler::new();
    handler.set_input_sender(Arc::new(move |block_id: &str, data: &[u8]| {
        sent_clone
            .lock()
            .unwrap()
            .push((block_id.to_string(), data.to_vec()));
        Ok(())
    }));
    handler
        .register_agent("agent1", "block1", None)
        .unwrap();

    let resp = handler.inject_message(InjectionRequest {
        target_agent: "agent1".to_string(),
        message: "hello".to_string(),
        source_agent: None,
        request_id: Some("req-1".to_string()),
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: None,
        forward_hops: 0,
    });

    assert!(resp.success);
    assert_eq!(resp.request_id, "req-1");
    assert_eq!(resp.block_id.as_deref(), Some("block1"));

    let calls = sent.lock().unwrap();
    // Production sequence (handler.rs:268-280): clear `\r`, then
    // message+`\r` as a single payload. The 3 delayed `\r` follow-ups
    // are tokio-spawned with 200ms delays so they don't run before
    // the assertions in this synchronous-only test body.
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], ("block1".to_string(), b"\r".to_vec()));
    // The message is now wrapped in a JEKT marker block (#1876); the block
    // carries a timestamp so assert structurally, not by exact bytes.
    assert_eq!(calls[1].0, "block1");
    let payload = String::from_utf8_lossy(&calls[1].1);
    assert!(payload.contains("[JEKT:"), "JEKT open marker present");
    assert!(payload.contains("[/JEKT]"), "JEKT close marker present");
    assert!(payload.contains("hello"), "message payload present");
    assert!(payload.ends_with('\r'), "trailing CR submits the message");
}

// Controller-aware delivery (SPEC_AGENT_CONTROL_PROTOCOL §6 / Phase 3).

// A structured (persistent/ACP) controller delivers via the message_sender and
// must NOT also emit PTY keystrokes through the input_sender.
#[test]
fn test_handler_inject_structured_delivery_skips_pty() {
    let pty_calls = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
    let msg_calls = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let pty_clone = pty_calls.clone();
    let msg_clone = msg_calls.clone();

    let mut handler = Handler::new();
    handler.set_input_sender(Arc::new(move |block_id: &str, data: &[u8]| {
        pty_clone
            .lock()
            .unwrap()
            .push((block_id.to_string(), data.to_vec()));
        Ok(())
    }));
    // Structured controller: report delivered (Ok(true)).
    handler.set_message_sender(Arc::new(move |block_id: &str, message: &str| {
        msg_clone
            .lock()
            .unwrap()
            .push((block_id.to_string(), message.to_string()));
        Ok(true)
    }));
    handler.register_agent("agent1", "block1", None).unwrap();

    let resp = handler.inject_message(InjectionRequest {
        target_agent: "agent1".to_string(),
        message: "hello".to_string(),
        source_agent: None,
        request_id: Some("req-1".to_string()),
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: None,
        forward_hops: 0,
    });

    assert!(resp.success);
    assert_eq!(resp.block_id.as_deref(), Some("block1"));
    // Structured channel got the (JEKT-wrapped, #1876) message; PTY got
    // nothing. The wrap carries a timestamp, so assert structurally.
    let msgs = msg_calls.lock().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].0, "block1");
    assert!(
        msgs[0].1.contains("[JEKT:")
            && msgs[0].1.contains("[/JEKT]")
            && msgs[0].1.contains("hello"),
        "structured channel got the JEKT-wrapped message (open + close markers + payload)"
    );
    assert!(pty_calls.lock().unwrap().is_empty());
}

// A PTY-based controller (message_sender returns Ok(false)) falls through to the
// keystroke path.
#[tokio::test]
async fn test_handler_inject_pty_fallback() {
    let pty_calls = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
    let pty_clone = pty_calls.clone();

    let mut handler = Handler::new();
    handler.set_input_sender(Arc::new(move |block_id: &str, data: &[u8]| {
        pty_clone
            .lock()
            .unwrap()
            .push((block_id.to_string(), data.to_vec()));
        Ok(())
    }));
    handler.set_message_sender(Arc::new(|_block_id: &str, _message: &str| Ok(false)));
    handler.register_agent("agent1", "block1", None).unwrap();

    let resp = handler.inject_message(InjectionRequest {
        target_agent: "agent1".to_string(),
        message: "hello".to_string(),
        source_agent: None,
        request_id: Some("req-1".to_string()),
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: None,
        forward_hops: 0,
    });

    assert!(resp.success);
    // Keystroke path ran: clear `\r` then `hello\r` (delayed `\r`s are spawned later).
    let calls = pty_calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], ("block1".to_string(), b"\r".to_vec()));
    // The message is now wrapped in a JEKT marker block (#1876); the block
    // carries a timestamp so assert structurally, not by exact bytes.
    assert_eq!(calls[1].0, "block1");
    let payload = String::from_utf8_lossy(&calls[1].1);
    assert!(payload.contains("[JEKT:"), "JEKT open marker present");
    assert!(payload.contains("[/JEKT]"), "JEKT close marker present");
    assert!(payload.contains("hello"), "message payload present");
    assert!(payload.ends_with('\r'), "trailing CR submits the message");
}

// A structured controller that fails to accept the message must surface the error
// and must NOT fall back to PTY keystrokes (persistent controllers reject them).
#[test]
fn test_handler_inject_structured_failure_no_pty_fallback() {
    let pty_calls = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
    let pty_clone = pty_calls.clone();

    let mut handler = Handler::new();
    handler.set_input_sender(Arc::new(move |block_id: &str, data: &[u8]| {
        pty_clone
            .lock()
            .unwrap()
            .push((block_id.to_string(), data.to_vec()));
        Ok(())
    }));
    handler.set_message_sender(Arc::new(|_block_id: &str, _message: &str| {
        Err("persistent process not running".to_string())
    }));
    handler.register_agent("agent1", "block1", None).unwrap();

    let resp = handler.inject_message(InjectionRequest {
        target_agent: "agent1".to_string(),
        message: "hello".to_string(),
        source_agent: None,
        request_id: Some("req-1".to_string()),
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: None,
        forward_hops: 0,
    });

    assert!(!resp.success);
    assert!(resp.error.unwrap().contains("persistent process not running"));
    // No PTY fallback for a structured controller.
    assert!(pty_calls.lock().unwrap().is_empty());
}

#[test]
fn test_handler_audit_log() {
    let mut handler = Handler::new();
    handler
        .register_agent("agent1", "block1", None)
        .unwrap();

    // Inject (will fail due to no sender)
    handler.inject_message(InjectionRequest {
        target_agent: "agent1".to_string(),
        message: "test".to_string(),
        source_agent: Some("src".to_string()),
        request_id: Some("req-1".to_string()),
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: None,
        forward_hops: 0,
    });

    let log = handler.get_audit_log(10);
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].target_agent, "agent1");
    assert_eq!(log[0].request_id, "req-1");
    assert!(!log[0].success);
}

/// Ordinary jekt injections (the only path that runs today) always leave
/// `outcome`/`reason` unset — those fields exist only for Warden Supervisor
/// entries, added via `log_audit`'s two new trailing params.
#[test]
fn test_handler_audit_log_ordinary_jekt_has_no_outcome() {
    let mut handler = Handler::new();
    handler.register_agent("agent1", "block1", None).unwrap();
    handler.inject_message(InjectionRequest {
        target_agent: "agent1".to_string(),
        message: "test".to_string(),
        source_agent: Some("src".to_string()),
        request_id: Some("req-1".to_string()),
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: None,
        forward_hops: 0,
    });

    let log = handler.get_audit_log(10);
    assert_eq!(log.len(), 1);
    assert!(log[0].outcome.is_none());
    assert!(log[0].reason.is_none());
}

/// `log_audit`'s new outcome/reason params, when set, land in the
/// resulting entry — this is the path `record_supervisor_decision` (a
/// follow-up PR) will call.
#[test]
fn test_log_audit_stores_outcome_and_reason_when_provided() {
    let mut handler = Handler::new();
    handler.log_audit(
        None,
        "agent1",
        "block1",
        "continue",
        true,
        None,
        "req-1",
        Some("nudge_sent"),
        Some("agent paused asking for permission to continue"),
    );

    let log = handler.get_audit_log(10);
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome.as_deref(), Some("nudge_sent"));
    assert_eq!(
        log[0].reason.as_deref(),
        Some("agent paused asking for permission to continue"),
    );
}

/// `AuditLogEntry`'s new fields must round-trip through serde and, per the
/// struct's `skip_serializing_if` convention (matching `source_agent`/
/// `error_message`), be OMITTED from the JSON entirely when `None` — a
/// back-compat guard for any existing consumer of the audit JSON shape.
#[test]
fn test_audit_log_entry_outcome_field_serde_roundtrip() {
    let with_outcome = AuditLogEntry {
        timestamp: 1,
        source_agent: None,
        target_agent: "agent1".to_string(),
        block_id: "block1".to_string(),
        message_hash: "hash".to_string(),
        message_length: 0,
        success: true,
        error_message: None,
        request_id: "req-1".to_string(),
        outcome: Some("nudge_declined".to_string()),
        reason: Some("consecutive-nudge ceiling reached".to_string()),
    };
    let json = serde_json::to_value(&with_outcome).unwrap();
    assert_eq!(json["outcome"], "nudge_declined");
    assert_eq!(json["reason"], "consecutive-nudge ceiling reached");
    let round_tripped: AuditLogEntry = serde_json::from_value(json).unwrap();
    assert_eq!(round_tripped.outcome.as_deref(), Some("nudge_declined"));

    let without_outcome = AuditLogEntry {
        outcome: None,
        reason: None,
        ..with_outcome
    };
    let json = serde_json::to_value(&without_outcome).unwrap();
    assert!(json.get("outcome").is_none(), "outcome must be omitted, not null, when None");
    assert!(json.get("reason").is_none(), "reason must be omitted, not null, when None");
}

#[test]
fn test_handler_audit_log_ring_buffer() {
    let mut handler = Handler::new();
    // Fill beyond capacity
    for i in 0..AUDIT_LOG_MAX + 10 {
        handler.log_audit(
            None,
            &format!("agent{}", i),
            "block",
            "msg",
            true,
            None,
            &format!("req-{}", i),
            None,
            None,
        );
    }

    let log = handler.get_audit_log(200);
    assert_eq!(log.len(), AUDIT_LOG_MAX);
    // Most recent first
    assert_eq!(log[0].request_id, "req-109");
}

// -- Poller tests --

#[test]
fn test_poller_status_unconfigured() {
    let handler = get_global_handler();
    let poller = Poller::new(
        PollerConfig {
            muxbus_url: None,
            muxbus_token: None,
            poll_interval_secs: 30,
        },
        handler,
    );

    let status = poller.status();
    assert!(!status.configured);
    assert!(!status.running);
}

#[test]
fn test_poller_status_configured() {
    let handler = get_global_handler();
    let poller = Poller::new(
        PollerConfig {
            muxbus_url: Some("https://example.com".to_string()),
            muxbus_token: Some("token123".to_string()),
            poll_interval_secs: 30,
        },
        handler,
    );

    let status = poller.status();
    assert!(status.configured);
    assert!(status.has_token);
}

#[test]
fn test_poller_record_poll() {
    let handler = get_global_handler();
    let poller = Poller::new(
        PollerConfig {
            muxbus_url: Some("https://example.com".to_string()),
            muxbus_token: Some("token123".to_string()),
            poll_interval_secs: 30,
        },
        handler,
    );

    poller.record_poll();
    poller.record_poll();
    poller.record_injections(5);

    let status = poller.status();
    assert_eq!(status.poll_count, 2);
    assert_eq!(status.injections_count, 5);
    assert!(status.last_poll.is_some());
}

#[test]
fn test_poller_reconfigure() {
    let handler = get_global_handler();
    let poller = Poller::new(
        PollerConfig {
            muxbus_url: None,
            muxbus_token: None,
            poll_interval_secs: 30,
        },
        handler,
    );

    assert!(!poller.is_configured());

    poller.reconfigure(
        Some("https://new.example.com".to_string()),
        Some("new-token".to_string()),
    );

    assert!(poller.is_configured());
    let status = poller.status();
    assert_eq!(status.url.as_deref(), Some("https://new.example.com"));
}

// -- Serde tests --

#[test]
fn test_injection_request_serde() {
    let req = InjectionRequest {
        target_agent: "Agent1".to_string(),
        message: "hello".to_string(),
        source_agent: Some("Agent2".to_string()),
        request_id: Some("req-123".to_string()),
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: None,
        forward_hops: 0,
    };

    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("target_agent"));
    assert!(json.contains("Agent1"));
    assert!(!json.contains("priority")); // None fields skipped

    let parsed: InjectionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.target_agent, "Agent1");
    assert_eq!(parsed.source_agent.as_deref(), Some("Agent2"));
}

#[test]
fn test_injection_response_serde() {
    let resp = InjectionResponse {
        success: true,
        request_id: "req-123".to_string(),
        block_id: Some("block-abc".to_string()),
        error: None,
        timestamp: 1700000000000,
        effective_tier: Some("coord".to_string()),
    };

    let json = serde_json::to_string(&resp).unwrap();
    let parsed: InjectionResponse = serde_json::from_str(&json).unwrap();
    assert!(parsed.success);
    assert_eq!(parsed.block_id.as_deref(), Some("block-abc"));
}

#[test]
fn test_pending_response_serde() {
    let json = r#"{"injections":[{"id":"inj-1","message":"hello","source_agent":"Agent2","created_at":1700000000000}]}"#;
    let parsed: PendingResponse = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.injections.len(), 1);
    assert_eq!(parsed.injections[0].id, "inj-1");
    assert_eq!(parsed.injections[0].message, "hello");
}

#[test]
fn test_agentmux_config_serde() {
    let config = AgentMuxConfigFile {
        url: Some("https://mux.example.com".to_string()),
        token: Some("secret".to_string()),
    };

    let json = serde_json::to_string(&config).unwrap();
    let parsed: AgentMuxConfigFile = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.url.as_deref(), Some("https://mux.example.com"));
}

// -- Thread-safe handler tests --

#[test]
fn test_reactive_handler_thread_safe() {
    let handler = ReactiveHandler::new();
    handler
        .register_agent("agent1", "block1", None)
        .unwrap();

    let agent = handler.get_agent("agent1").unwrap();
    assert_eq!(agent.block_id, "block1");

    handler.unregister_agent("agent1");
    assert!(handler.get_agent("agent1").is_none());
}

#[test]
fn test_reactive_handler_list() {
    let handler = ReactiveHandler::new();
    handler
        .register_agent("a1", "b1", None)
        .unwrap();
    handler
        .register_agent("a2", "b2", Some("t2"))
        .unwrap();

    let agents = handler.list_agents();
    assert_eq!(agents.len(), 2);
}

// -- InjectionRequest::forward_hops --

#[test]
fn injection_request_forward_hops_defaults_to_zero_when_absent() {
    // Older callers (muxbus client, any payload predating this field) must
    // still deserialize -- the hop-count guard added for PR #2350's forward-
    // loop fix must not break existing cross-instance callers.
    let json = serde_json::json!({
        "target_agent": "agent1",
        "message": "hello",
    });
    let req: InjectionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.forward_hops, 0);
}

#[test]
fn injection_request_forward_hops_round_trips() {
    let json = serde_json::json!({
        "target_agent": "agent1",
        "message": "hello",
        "forward_hops": 2,
    });
    let req: InjectionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.forward_hops, 2);
}
