// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn shutdown_pending_is_immediate_attention_and_not_silenceable_per_kind() {
    let k = NotifyKind::ShutdownPending;
    assert_eq!(k.setting_suffix(), None, "only the master switch can silence it");
    assert_eq!(k.delay_ms(), 0, "the window is short");
    assert!(matches!(k.priority(), Priority::Attention));
    assert_eq!(k.group_key("b1"), "shutdown:b1");
    assert_eq!(title_for(k, "Camper"), "Camper will shut down soon");
}

fn req(kind: NotifyKind, block: &str) -> Request {
    Request { kind, block_id: block.into(), agent_name: "lark".into(), body: Some("Which branch?".into()), summary: None, shutdown: None }
}
fn focus(window: bool, block: Option<&str>) -> Input {
    Input::Focus { conn_id: "c1".into(), report: FocusReport { window_focused: window, block_id: block.map(Into::into), window_id: None } }
}
fn shows(a: &[Action]) -> Vec<&Notification> {
    a.iter().filter_map(|x| if let Action::Show(n) = x { Some(n) } else { None }).collect()
}
fn retracts(a: &[Action]) -> usize {
    a.iter().filter(|x| matches!(x, Action::Retract { .. })).count()
}

#[test]
fn cloud_signed_out_shows_at_once_even_while_focused_and_retracts_when_resolved() {
    let k = NotifyKind::CloudSignedOut;
    assert_eq!(k.setting_suffix(), Some("cloudsignedout"));
    assert_eq!(title_for(k, ""), "AgentMux is signed out of MuxBus");
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(focus(true, Some("b1")), 0, &s);
    let blockless = Request { kind: k, block_id: String::new(), agent_name: String::new(), body: Some("x".into()), summary: Some("s".into()), shutdown: None };
    p.step(Input::Emit(blockless), 0, &s);
    let a = p.step(Input::Tick, 0, &s);
    let n = shows(&a);
    assert_eq!(n.len(), 1, "attention reaches a user looking at a pane");
    assert_eq!(n[0].summary, None, "about no agent");
    assert_eq!(retracts(&p.step(Input::Resolve { block_id: String::new(), family: Family::Turn }, 1, &s)), 0);
    assert_eq!(retracts(&p.step(Input::Resolve { block_id: String::new(), family: Family::Cloud }, 1, &s)), 1);
}

/// One per account episode, keyed by the account, taken down when it ends
/// (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6.2).
#[test]
fn an_account_episode_has_one_toast_per_account_until_it_ends() {
    let k = NotifyKind::AccountUnavailable;
    assert_eq!(k.setting_suffix(), Some("accountunavailable"));
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(focus(true, Some("b1")), 0, &s);
    let account = |key: &str| Request { kind: k, block_id: key.into(), agent_name: String::new(), body: Some("2 agents are out of quota".into()), summary: Some("s".into()), shutdown: None };
    p.step(Input::Emit(account("account:aaaa")), 0, &s);
    p.step(Input::Emit(account("account:bbbb")), 0, &s);
    let a = p.step(Input::Tick, 0, &s);
    let n = shows(&a);
    assert_eq!(n.len(), 2, "two accounts, two toasts, shown at once while focused");
    assert_eq!((n[0].title.as_str(), n[0].summary.as_deref()), ("An account can't run your agents", None));
    assert_ne!(n[0].tag, n[1].tag);
    assert_eq!(retracts(&p.step(Input::Resolve { block_id: "account:aaaa".into(), family: Family::Turn }, 1, &s)), 0);
    assert_eq!(retracts(&p.step(Input::Resolve { block_id: "account:aaaa".into(), family: Family::Account }, 1, &s)), 1);
    assert_eq!(p.tray_state(&s, 1).attention.len(), 1, "the other account's is still up");
    let off = crate::backend::notify::router::settings_from_extra(&std::collections::HashMap::from([(
        "notify:os:accountunavailable".to_string(),
        serde_json::json!(false),
    )]));
    let mut q = PolicyState::new();
    q.step(Input::Emit(account("account:cccc")), 0, &off);
    assert!(shows(&q.step(Input::Tick, 0, &off)).is_empty(), "its own setting turns it off");
}

#[test]
fn debounces_then_shows_when_app_unfocused() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(focus(false, None), 0, &s);
    assert!(p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &s).is_empty());
    assert!(shows(&p.step(Input::Tick, 9_999, &s)).is_empty());
    let a = p.step(Input::Tick, 10_000, &s);
    let n = shows(&a);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].title, "lark finished");
    // Already shown → not shown again on later ticks.
    assert!(p.step(Input::Tick, 20_000, &s).is_empty());
}

#[test]
fn resolve_inside_debounce_drops_silently() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &s);
    let a = p.step(Input::Resolve { block_id: "b1".into(), family: Family::Input }, 3_000, &s);
    assert!(a.is_empty(), "never shown → nothing to retract");
    assert!(p.step(Input::Tick, 10_000, &s).is_empty());
    assert_eq!(p.live_len(), 0);
}

#[test]
fn resolve_after_show_retracts() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &s);
    assert_eq!(shows(&p.step(Input::Tick, 6_000, &s)).len(), 1);
    let a = p.step(Input::Resolve { block_id: "b1".into(), family: Family::Input }, 7_000, &s);
    assert_eq!(retracts(&a), 1);
}

#[test]
fn looking_at_the_block_suppresses_even_attention() {
    let s = Settings { when: When::Always, ..Settings::default() };
    let mut p = PolicyState::new();
    p.step(focus(true, Some("b1")), 0, &s);
    p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &s);
    assert!(shows(&p.step(Input::Tick, 6_000, &s)).is_empty());
}

#[test]
fn unfocused_mode_suppresses_normal_but_not_attention_when_app_focused_elsewhere() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(focus(true, Some("other")), 0, &s);
    p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &s);
    p.step(Input::Emit(req(NotifyKind::InputWaiting, "b2")), 0, &s);
    let a = p.step(Input::Tick, 10_000, &s);
    let n = shows(&a);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].kind, NotifyKind::InputWaiting);
}

#[test]
fn focusing_the_block_retracts_its_shown_toasts() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &s);
    assert_eq!(shows(&p.step(Input::Tick, 10_000, &s)).len(), 1);
    let a = p.step(focus(true, Some("b1")), 11_000, &s);
    assert_eq!(retracts(&a), 1);
    assert_eq!(p.live_len(), 0);
}

#[test]
fn same_family_replaces_in_place_with_same_tag() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &s);
    let first = shows(&p.step(Input::Tick, 10_000, &s))[0].clone();
    // New turn errors: old shown toast is pulled, new one debounces.
    let a = p.step(Input::Emit(req(NotifyKind::TurnErrored, "b1")), 12_000, &s);
    assert_eq!(retracts(&a), 1);
    let second = shows(&p.step(Input::Tick, 22_000, &s))[0].clone();
    assert_eq!(first.tag, second.tag);
    assert_ne!(first.id, second.id);
    assert_eq!(second.title, "lark stopped with an error");
    assert_eq!(p.live_len(), 1);
}

#[test]
fn disabled_master_or_kind_or_never_mode_shows_nothing() {
    let mut p = PolicyState::new();
    let off = Settings { enabled: false, ..Settings::default() };
    p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &off);
    assert_eq!(p.live_len(), 0);

    let mut kinds = HashMap::new();
    kinds.insert(NotifyKind::TurnCompleted, false);
    let kind_off = Settings { kind_enabled: kinds, ..Settings::default() };
    p.step(Input::Emit(req(NotifyKind::TurnCompleted, "b1")), 0, &kind_off);
    assert_eq!(p.live_len(), 0);

    let never = Settings { when: When::Never, ..Settings::default() };
    p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &never);
    assert!(shows(&p.step(Input::Tick, 6_000, &never)).is_empty());
}

#[test]
fn test_kind_is_immediate_and_bypasses_gates() {
    let s = Settings { enabled: false, when: When::Never, ..Settings::default() };
    let mut p = PolicyState::new();
    p.step(focus(true, Some("")), 0, &s);
    let a = p.step(Input::Emit(Request { kind: NotifyKind::Test, block_id: String::new(), agent_name: String::new(), body: None, summary: None, shutdown: None }), 0, &s);
    assert_eq!(shows(&a).len(), 1);
    assert_eq!(shows(&a)[0].title, "AgentMux notifications are working");
}

#[test]
fn click_ack_retracts_and_activates_dismiss_only_retracts() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &s);
    let id = shows(&p.step(Input::Tick, 6_000, &s))[0].id.clone();
    let a = p.step(Input::Ack { id: id.clone(), clicked: true }, 7_000, &s);
    assert_eq!(retracts(&a), 1);
    assert!(a.contains(&Action::Activate { id, block_id: "b1".into() }));

    p.step(Input::Emit(req(NotifyKind::InputWaiting, "b2")), 8_000, &s);
    let id2 = shows(&p.step(Input::Tick, 14_000, &s))[0].id.clone();
    let a = p.step(Input::Ack { id: id2, clicked: false }, 15_000, &s);
    assert_eq!(retracts(&a), 1);
    assert!(!a.iter().any(|x| matches!(x, Action::Activate { .. })));
}

#[test]
fn unknown_ack_is_a_noop() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    assert!(p.step(Input::Ack { id: "forged".into(), clicked: true }, 0, &s).is_empty());
}

#[test]
fn preview_none_strips_body() {
    let s = Settings { preview: Preview::None, ..Settings::default() };
    let mut p = PolicyState::new();
    p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 0, &s);
    assert_eq!(shows(&p.step(Input::Tick, 6_000, &s))[0].body, None);
}

#[test]
fn tag_is_stable_and_short() {
    assert_eq!(tag_for("input:b1"), tag_for("input:b1"));
    assert_ne!(tag_for("input:b1"), tag_for("turn:b1"));
    assert_eq!(tag_for("x").len(), 16);
}
fn req_b(kind: NotifyKind, block: &str) -> Request {
    Request { kind, block_id: block.into(), agent_name: "lark".into(), body: None, summary: None, shutdown: None }
}

#[test]
fn crash_outranks_later_generic_error_in_same_group() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(Request { kind: NotifyKind::AgentCrashed, block_id: "b1".into(), agent_name: "lark".into(), body: Some("Needs you to sign in again".into()), summary: None, shutdown: None }), 0, &s);
    p.step(Input::Emit(req_b(NotifyKind::TurnErrored, "b1")), 500, &s);
    let a = p.step(Input::Tick, 10_000, &s);
    let n = shows(&a);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].kind, NotifyKind::AgentCrashed);
    assert_eq!(n[0].title, "lark stopped unexpectedly");
    assert_eq!(n[0].body.as_deref(), Some("Needs you to sign in again"));
}

#[test]
fn generic_error_is_upgraded_by_crash() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req_b(NotifyKind::TurnErrored, "b1")), 0, &s);
    p.step(Input::Emit(req_b(NotifyKind::AgentCrashed, "b1")), 500, &s);
    let n = shows(&p.step(Input::Tick, 10_500, &s)).into_iter().cloned().collect::<Vec<_>>();
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].kind, NotifyKind::AgentCrashed);
}

#[test]
fn pause_holds_back_everything_but_optionally_attention() {
    let mut s = Settings { pause_until_ms: 100_000, ..Settings::default() };
    let mut p = PolicyState::new();
    p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b1")), 0, &s);
    p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b2")), 0, &s);
    assert!(shows(&p.step(Input::Tick, 20_000, &s)).is_empty());

    s.pause_allow_attention = true;
    p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b3")), 30_000, &s);
    p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b4")), 30_000, &s);
    let a = p.step(Input::Tick, 45_000, &s);
    let n = shows(&a);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].block_id, "b4");

    // Pause expired → normal again.
    p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b5")), 100_000, &s);
    assert_eq!(shows(&p.step(Input::Tick, 110_000, &s)).len(), 1);
}

#[test]
fn a_pending_shutdown_gets_through_a_pause_and_quiet_hours() {
    for s in [
        Settings { pause_until_ms: 100_000, ..Settings::default() },
        Settings { quiet_until_ms: 100_000, ..Settings::default() },
    ] {
        assert!(!s.pause_allow_attention, "the default, which held it back");
        let mut p = PolicyState::new();
        p.step(Input::Emit(req_b(NotifyKind::ShutdownPending, "b1")), 0, &s);
        let a = p.step(Input::Tick, 0, &s);
        assert_eq!(shows(&a).len(), 1, "{s:?}");
        assert_eq!(shows(&a)[0].kind, NotifyKind::ShutdownPending);
    }
    // The master switch still silences it.
    let off = Settings { enabled: false, pause_until_ms: 100_000, ..Settings::default() };
    let mut p = PolicyState::new();
    p.step(Input::Emit(req_b(NotifyKind::ShutdownPending, "b1")), 0, &off);
    assert!(shows(&p.step(Input::Tick, 0, &off)).is_empty());
}

#[test]
fn a_pending_shutdown_is_not_folded_by_the_global_rate_cap() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    for i in 0..GLOBAL_MAX {
        p.step(Input::Emit(req_b(NotifyKind::InputWaiting, &format!("b{i}"))), 0, &s);
    }
    assert_eq!(shows(&p.step(Input::Tick, 6_000, &s)).len(), GLOBAL_MAX, "the cap is now full");
    p.step(Input::Emit(req_b(NotifyKind::ShutdownPending, "b9")), 7_000, &s);
    let a = p.step(Input::Tick, 7_000, &s);
    let n = shows(&a);
    assert_eq!(n.len(), 1, "shown itself, not folded into the summary: {n:?}");
    assert_eq!(n[0].kind, NotifyKind::ShutdownPending);
}

#[test]
fn burst_folds_into_one_summary_and_focus_clears_it() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    for i in 0..20 {
        p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, &format!("b{i:02}"))), i, &s);
    }
    let a = p.step(Input::Tick, 10_100, &s);
    let n = shows(&a);
    let normal: Vec<_> = n.iter().filter(|x| x.kind == NotifyKind::TurnCompleted).collect();
    let summaries: Vec<_> = n.iter().filter(|x| x.kind == NotifyKind::Summary).collect();
    assert_eq!(normal.len(), GLOBAL_MAX, "global limit");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].title, "16 more agent updates");

    // A second burst replaces the same summary (same tag), counting up.
    p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "c1")), 11_000, &s);
    let n2 = shows(&p.step(Input::Tick, 21_000, &s)).into_iter().cloned().collect::<Vec<_>>();
    assert_eq!(n2.len(), 1);
    assert_eq!(n2[0].title, "17 more agent updates");
    assert_eq!(n2[0].tag, summaries[0].tag);

    // Focusing the app retracts the summary and resets the count.
    let a = p.step(focus(true, None), 22_000, &s);
    assert_eq!(retracts(&a), 1);
    p.step(focus(false, None), 23_000, &s);
    for i in 0..6 {
        p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, &format!("d{i}"))), 100_000 + i, &s);
    }
    let n3 = shows(&p.step(Input::Tick, 110_100, &s)).into_iter().cloned().collect::<Vec<_>>();
    assert!(n3.iter().any(|x| x.title == "2 more agent updates"), "{n3:?}");
}

#[test]
fn per_block_limit_spares_attention() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b1")), 0, &s);
    assert_eq!(shows(&p.step(Input::Tick, 10_000, &s)).len(), 1);
    // Same block, 5s later: a Normal would be limited, Attention is not.
    p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b1")), 9_000, &s);
    let a = p.step(Input::Tick, 15_000, &s);
    assert_eq!(shows(&a).len(), 1);
    assert_eq!(shows(&a)[0].kind, NotifyKind::InputWaiting);
}

#[test]
fn quiet_hours_hold_back_like_a_pause_but_are_not_shown_as_one() {
    let s = Settings { quiet_until_ms: 100_000, ..Settings::default() };
    let mut p = PolicyState::new();
    p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b1")), 0, &s);
    assert!(shows(&p.step(Input::Tick, 20_000, &s)).is_empty());
    assert_eq!(p.tray_state(&s, 20_000).paused_until_ms, 0);
}

#[test]
fn tray_state_lists_shown_attention_only() {
    let s = Settings { pause_until_ms: 1_000_000, pause_allow_attention: true, ..Settings::default() };
    let mut p = PolicyState::new();
    p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b1")), 0, &s);
    p.step(Input::Emit(req_b(NotifyKind::TurnCompleted, "b2")), 0, &s);
    assert!(p.tray_state(&s, 1_000).attention.is_empty(), "pending isn't shown yet");
    p.step(Input::Tick, 10_000, &s);
    let t = p.tray_state(&s, 10_000);
    assert_eq!(t.attention.len(), 1);
    assert_eq!(t.attention[0].block_id, "b1");
    assert_eq!(t.attention[0].title, "lark has a question");
    assert_eq!(t.attention[0].kind, NotifyKind::InputWaiting);
    assert_eq!(t.paused_until_ms, 1_000_000);
    assert_eq!(p.tray_state(&s, 2_000_000).paused_until_ms, 0);
}

#[test]
fn repeated_same_kind_report_is_idempotent() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    // srv detects the question (no text), then the renderer reports it too.
    p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b1")), 0, &s);
    let a = p.step(Input::Tick, 6_000, &s);
    let first = shows(&a)[0].clone();
    let a = p.step(Input::Emit(req(NotifyKind::InputWaiting, "b1")), 7_000, &s);
    assert!(a.is_empty(), "no retract, no new toast: {a:?}");
    assert!(p.step(Input::Tick, 20_000, &s).is_empty(), "not re-debounced/re-shown");
    let t = p.tray_state(&s, 20_000);
    assert_eq!(t.attention.len(), 1);
    assert_eq!(t.attention[0].id, first.id, "same notification stays live");

    // Pending (not yet shown) duplicate keeps its original due time and
    // picks up the body it was missing.
    p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b2")), 30_000, &s);
    p.step(Input::Emit(req(NotifyKind::InputWaiting, "b2")), 35_000, &s);
    let n = shows(&p.step(Input::Tick, 36_000, &s)).into_iter().cloned().collect::<Vec<_>>();
    assert_eq!(n.len(), 1, "due at 36s from the FIRST report");
    assert_eq!(n[0].body.as_deref(), Some("Which branch?"));
}

#[test]
fn a_second_crash_is_news_not_a_duplicate() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req_b(NotifyKind::AgentCrashed, "b1")), 0, &s);
    let first = shows(&p.step(Input::Tick, 10_000, &s))[0].clone();
    let a = p.step(Input::Emit(req_b(NotifyKind::AgentCrashed, "b1")), 60_000, &s);
    assert_eq!(retracts(&a), 1, "old toast pulled for the new crash");
    let second = shows(&p.step(Input::Tick, 70_000, &s))[0].clone();
    assert_ne!(first.id, second.id);
    assert_eq!(first.tag, second.tag);
}

fn req_s(kind: NotifyKind, block: &str, summary: Option<&str>) -> Request {
    Request { summary: summary.map(Into::into), ..req(kind, block) }
}

#[test]
fn summary_rides_along_and_is_dropped_by_its_setting_or_preview_none() {
    let mut p = PolicyState::new();
    let s = Settings::default();
    p.step(Input::Emit(req_s(NotifyKind::InputWaiting, "b1", Some("Fix resize repaint"))), 0, &s);
    let n = shows(&p.step(Input::Tick, 6_000, &s))[0].clone();
    assert_eq!(n.title, "lark has a question");
    assert_eq!(n.body.as_deref(), Some("Which branch?"));
    assert_eq!(n.summary.as_deref(), Some("Fix resize repaint"));

    for off in [
        Settings { show_summary: false, ..Settings::default() },
        Settings { preview: Preview::None, ..Settings::default() },
    ] {
        let mut p = PolicyState::new();
        p.step(Input::Emit(req_s(NotifyKind::TurnCompleted, "b1", Some("Fix resize repaint"))), 0, &off);
        assert_eq!(shows(&p.step(Input::Tick, 10_000, &off))[0].summary, None, "{off:?}");
    }
}

#[test]
fn duplicate_question_report_fills_a_missing_summary() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    // srv sees the question first (no pane → no summary read yet), then
    // the renderer reports it with the block's summary set.
    p.step(Input::Emit(req_s(NotifyKind::InputWaiting, "b1", None)), 0, &s);
    p.step(Input::Emit(req_s(NotifyKind::InputWaiting, "b1", Some("Fix resize repaint"))), 1_000, &s);
    let n = shows(&p.step(Input::Tick, 6_000, &s))[0].clone();
    assert_eq!(n.summary.as_deref(), Some("Fix resize repaint"));

    let off = Settings { show_summary: false, ..Settings::default() };
    let mut p = PolicyState::new();
    p.step(Input::Emit(req_s(NotifyKind::InputWaiting, "b1", None)), 0, &off);
    p.step(Input::Emit(req_s(NotifyKind::InputWaiting, "b1", Some("x"))), 1_000, &off);
    assert_eq!(shows(&p.step(Input::Tick, 6_000, &off))[0].summary, None);
}

#[test]
fn summary_line_is_left_out_on_the_digest_and_the_test_toast() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    let a = p.step(Input::Emit(req_s(NotifyKind::Test, "", Some("x"))), 0, &s);
    assert_eq!(shows(&a)[0].summary, None);
}

#[test]
fn serialized_payload_omits_an_absent_summary() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req_s(NotifyKind::InputWaiting, "b1", None)), 0, &s);
    let n = shows(&p.step(Input::Tick, 6_000, &s))[0].clone();
    let v = serde_json::to_value(&n).unwrap();
    assert!(v.get("summary").is_none(), "an older launcher sees the same shape as before: {v}");
    // And a payload without the field still deserializes.
    let back: Notification = serde_json::from_value(v).unwrap();
    assert_eq!(back.summary, None);
}

fn focus_as(conn: &str, window_id: &str, focused: bool) -> Input {
    Input::Focus {
        conn_id: conn.into(),
        report: FocusReport { window_focused: focused, block_id: None, window_id: Some(window_id.into()) },
    }
}

#[test]
fn open_windows_and_the_raise_target_follow_focus_reports() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    assert!(!p.window_open("w1"));
    assert_eq!(p.raise_window(), None);
    p.step(focus_as("c1", "w1", false), 0, &s);
    p.step(focus_as("c2", "w2", false), 0, &s);
    assert!(p.window_open("w1") && p.window_open("w2"));
    assert!(!p.window_open("w3"));
    // Nobody focused yet: some open window, deterministically.
    assert_eq!(p.raise_window().as_deref(), Some("w1"));
    p.step(focus_as("c2", "w2", true), 1, &s);
    p.step(focus_as("c2", "w2", false), 2, &s);
    assert_eq!(p.raise_window().as_deref(), Some("w2"), "last focused, even after it blurred");
    p.step(Input::Disconnect { conn_id: "c2".into() }, 3, &s);
    assert!(!p.window_open("w2"));
    assert_eq!(p.raise_window().as_deref(), Some("w1"));
}

#[test]
fn needs_review_is_attention_with_its_own_group() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req_b(NotifyKind::MessageNeedsReview, "b1")), 0, &s);
    p.step(Input::Emit(req_b(NotifyKind::InputWaiting, "b1")), 0, &s);
    let a = p.step(Input::Tick, 6_000, &s);
    assert_eq!(shows(&a).len(), 2);
    assert!(shows(&a).iter().any(|n| n.title == "A message for lark needs your review"));
}

fn shutdown_req(effect: &'static str, running: bool, deadline_ms: i64) -> Request {
    Request {
        shutdown: Some(ShutdownInfo { deadline_ms, effect, running, request_id: "r1".into(), labels: None }),
        ..req(NotifyKind::ShutdownPending, "b1")
    }
}

#[test]
fn a_pending_shutdown_says_what_will_happen_counts_down_and_offers_keep_and_close_now() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(shutdown_req("close_tab", true, 16_000)), 1_000, &s);
    let a = p.step(Input::Tick, 1_000, &s);
    let n = &shows(&a)[0];
    assert_eq!(n.title, "Closing lark in 15 s");
    assert_eq!(n.actions, Some(vec![NotifyAction { id: "keep".into(), label: "Keep running".into() }, NotifyAction { id: "now".into(), label: "Close now".into() }]));
    assert_eq!(n.countdown, Some(Countdown { remaining_ms: 15_000, title_template: "Closing lark in {n} s".into() }));
}

#[test]
fn a_sensitive_operation_counts_down_with_its_name_and_its_own_two_buttons() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    let sensitive = Request {
        shutdown: Some(ShutdownInfo { deadline_ms: 31_000, effect: SENSITIVE_EFFECT, running: true, request_id: "r9".into(), labels: Some(("Don't upgrade".into(), "Upgrade now".into())) }),
        agent_name: "Upgrade AgentMux".into(),
        block_id: "sensitive:r9".into(),
        ..req(NotifyKind::ShutdownPending, "sensitive:r9")
    };
    p.step(Input::Emit(sensitive), 1_000, &s);
    let a = p.step(Input::Tick, 1_000, &s);
    let n = &shows(&a)[0];
    assert_eq!(n.title, "Upgrade AgentMux in 30 s");
    assert_eq!(n.actions, Some(vec![NotifyAction { id: "keep".into(), label: "Don't upgrade".into() }, NotifyAction { id: "now".into(), label: "Upgrade now".into() }]), "its own words");
    assert_eq!(n.countdown, Some(Countdown { remaining_ms: 30_000, title_template: "Upgrade AgentMux in {n} s".into() }));
    assert_eq!(p.live_by_id(&n.id), Some((NotifyKind::ShutdownPending, "sensitive:r9".to_string(), Some("r9".to_string()))));
}

#[test]
fn the_button_and_the_verb_follow_the_agent_and_the_effect() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(shutdown_req("close_tab", false, 10_000)), 0, &s);
    let a = p.step(Input::Tick, 0, &s);
    assert_eq!(shows(&a)[0].actions.as_ref().unwrap()[0].label, "Keep open", "nothing is running to keep");
    assert_eq!(shows(&a)[0].actions.as_ref().unwrap()[1].label, "Close now");
    let mut p = PolicyState::new();
    p.step(Input::Emit(shutdown_req("stop", true, 8_500)), 0, &s);
    let a = p.step(Input::Tick, 0, &s);
    assert_eq!(shows(&a)[0].title, "Stopping lark in 9 s", "whole seconds, rounded up");
    assert_eq!(shows(&a)[0].actions.as_ref().unwrap()[1].label, "Stop now");
    let mut p = PolicyState::new();
    p.step(Input::Emit(shutdown_req("close_pane", true, 100)), 5_000, &s);
    let a = p.step(Input::Tick, 5_000, &s);
    let n = &shows(&a)[0];
    assert_eq!((n.title.as_str(), n.countdown.as_ref().unwrap().remaining_ms), ("Closing lark in 0 s", 0), "never negative");
}

/// The two emissions for an upgraded request run on separate threads: the
/// weaker one arriving last must not put "Stopping" back (#5397).
#[test]
fn a_late_weaker_emission_for_the_same_request_does_not_replace_a_stronger_one() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(shutdown_req("close_tab", true, 15_000)), 0, &s);
    p.step(Input::Emit(shutdown_req("stop", true, 15_000)), 1, &s);
    let a = p.step(Input::Tick, 1, &s);
    assert_eq!(shows(&a)[0].title, "Closing lark in 15 s", "the stale 'Stopping' was dropped");
    // In order, the stronger replaces the weaker as before.
    let mut p = PolicyState::new();
    p.step(Input::Emit(shutdown_req("stop", true, 15_000)), 0, &s);
    p.step(Input::Emit(shutdown_req("close_tab", true, 15_000)), 1, &s);
    let a = p.step(Input::Tick, 1, &s);
    assert_eq!(shows(&a)[0].title, "Closing lark in 15 s");
    // A different request for the block is a new shutdown, whatever its effect.
    let newer = Request {
        shutdown: Some(ShutdownInfo { deadline_ms: 30_000, effect: "stop", running: true, request_id: "r2".into(), labels: None }),
        ..req(NotifyKind::ShutdownPending, "b1")
    };
    p.step(Input::Emit(newer), 2, &s);
    let a = p.step(Input::Tick, 2, &s);
    assert!(shows(&a)[0].title.starts_with("Stopping lark"), "{:?}", shows(&a)[0].title);
}

#[test]
fn an_agent_name_holding_the_placeholder_keeps_its_name_in_the_first_title() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    let r = Request { agent_name: "x{n}".into(), ..shutdown_req("close_tab", true, 9_000) };
    p.step(Input::Emit(r), 0, &s);
    let a = p.step(Input::Tick, 0, &s);
    let n = &shows(&a)[0];
    assert_eq!(n.title, "Closing x{n} in 9 s");
    assert_eq!(n.countdown.as_ref().unwrap().title_template, "Closing x{n} in {n} s");
}

#[test]
fn only_a_pending_shutdown_carries_buttons_or_a_countdown() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(req(NotifyKind::InputWaiting, "b3")), 0, &s);
    let due = p.step(Input::Tick, 10_000, &s);
    let n = &shows(&due)[0];
    assert!(n.actions.is_none() && n.countdown.is_none());
    // A shutdown with no info (an older caller) keeps the old fixed title.
    let mut p = PolicyState::new();
    p.step(Input::Emit(req(NotifyKind::ShutdownPending, "b4")), 0, &s);
    let a = p.step(Input::Tick, 0, &s);
    let n = &shows(&a)[0];
    assert_eq!(n.title, "lark will shut down soon");
    assert!(n.actions.is_none() && n.countdown.is_none());
}

#[test]
fn a_press_is_looked_up_by_the_notification_not_taken_from_the_press() {
    let s = Settings::default();
    let mut p = PolicyState::new();
    p.step(Input::Emit(shutdown_req("close_tab", true, 15_000)), 0, &s);
    let a = p.step(Input::Tick, 0, &s);
    let id = shows(&a)[0].id.clone();
    assert_eq!(p.live_by_id(&id), Some((NotifyKind::ShutdownPending, "b1".to_string(), Some("r1".to_string()))));
    assert_eq!(p.live_by_id("n0-nope"), None);
    p.step(Input::Ack { id: id.clone(), clicked: false }, 1_000, &s);
    assert_eq!(p.live_by_id(&id), None, "gone once acked, so a late press finds nothing");
}
