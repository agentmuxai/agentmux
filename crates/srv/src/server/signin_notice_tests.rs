// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::server::tests::test_state;

#[test]
fn only_the_providers_own_https_pages_are_links() {
    for ok in ["https://claude.com/cai/oauth/authorize?code=true&client_id=x", "https://claude.ai/login", "https://console.anthropic.com/oauth"] {
        assert!(link_allowed("claude", ok), "{ok}");
    }
    for bad in [
        "http://claude.com/x",
        "https://claude.com.evil.example/x",
        "https://evilclaude.com/x",
        "https://user:pw@claude.com/x",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "claude.com/x",
        "",
    ] {
        assert!(!link_allowed("claude", bad), "{bad}");
    }
    assert!(link_allowed("codex", "https://auth.openai.com/codex/device"));
    assert!(!link_allowed("codex", "https://claude.com/x"), "each provider has its own hosts");
    assert!(!link_allowed("gemini", "https://claude.com/x"), "a provider with no hosts listed accepts nothing");
}

#[test]
fn a_code_is_short_letters_digits_and_dashes() {
    assert_eq!(clean_code(Some(" RC63-52E85 ")).as_deref(), Some("RC63-52E85"));
    for bad in ["", "RC63 52E85", "<b>x</b>", &"A".repeat(40)] {
        assert_eq!(clean_code(Some(bad)), None, "{bad}");
    }
    assert_eq!(clean_code(None), None);
}

#[tokio::test]
async fn a_notice_is_shown_only_for_a_real_link() {
    let state = test_state();
    assert!(show_local(&state, "claude", "narko", "https://claude.com/cai/oauth/authorize", None).is_ok());
    assert!(show_local(&state, "claude", "narko", "https://evil.example/x", None).is_err());
}

#[tokio::test]
async fn with_no_window_focused_and_no_peer_the_link_is_shown_here() {
    let state = test_state();
    assert!(!window_focused(&state));
    let shown_on = route(&state, "agenty", "claude", "charlie", "https://claude.com/cai/oauth/authorize", None).await.unwrap();
    assert_eq!(shown_on, state.hostname);
    assert!(route(&state, "agenty", "claude", "charlie", "https://evil.example/x", None).await.is_err());
}

#[tokio::test]
async fn the_notice_op_is_a_write_and_the_activity_op_a_read() {
    use super::super::remote_ops::lookup;
    let notice = lookup("identity.signin.notice").unwrap();
    assert!(notice.changes_state && notice.needs_approved_install);
    let activity = lookup("app.activity").unwrap();
    assert!(!activity.changes_state);
}
