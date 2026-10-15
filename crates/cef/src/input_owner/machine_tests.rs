// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The input owner's transition table (spec §3.3), the gaps it closes
//! (REPORT_BROWSER_PANE_INPUT_FOCUS_STATE_2026_10_15.md §3.2), and its
//! invariants (spec §3.4) over random event sequences.

use super::*;
use proptest::prelude::*;

fn pane(b: &str) -> Owner {
    Owner::Pane(b.to_string())
}

fn at(b: &str) -> Surface {
    Surface::Pane(b.to_string())
}

/// An active window with live panes `a` and `b`, the keyboard with the app.
fn window() -> WindowInput {
    let mut w = WindowInput::default();
    w.step(Event::PaneLive("a".into()), 0);
    w.step(Event::PaneLive("b".into()), 0);
    w.step(Event::Activated, 0);
    w.step(Event::Verify(Surface::App), 50);
    assert_eq!(w.have, Have::Owned(Surface::App));
    w
}

/// `w` with the keyboard settled on pane `b`.
fn owned_by(mut w: WindowInput, b: &str, now: u64) -> WindowInput {
    w.step(Event::Select(pane(b)), now);
    w.step(Event::FocusGained(at(b)), now + 1);
    assert_eq!(w.have, Have::Owned(at(b)));
    w
}

fn focus_effects(effects: &[Effect]) -> Vec<&Effect> {
    effects.iter().filter(|e| matches!(e, Effect::FocusApp | Effect::FocusPane(_))).collect()
}

// ── Select ─────────────────────────────────────────────────────────────

#[test]
fn a_selection_asks_for_the_pane_and_checks_it_arrived() {
    let mut w = window();
    let fx = w.step(Event::Select(pane("a")), 100);
    assert_eq!(fx, vec![Effect::FocusPane("a".into()), Effect::Verify { after_ms: VERIFY_AFTER_MS }]);
    assert_eq!(w.have, Have::Pending { target: pane("a"), attempt: 0 });
    assert!(w.step(Event::FocusGained(at("a")), 101).is_empty());
    assert_eq!(w.have, Have::Owned(at("a")));
}

#[test]
fn repeated_selections_ask_once() {
    // G4: four requests in 10 ms produced one WM_SETFOCUS and left the global flag armed.
    let mut w = window();
    let mut asks = 0;
    for t in 0..4 {
        asks += focus_effects(&w.step(Event::Select(pane("a")), 100 + t)).len();
    }
    assert_eq!(asks, 1);
}

#[test]
fn selecting_the_owner_only_checks() {
    let mut w = owned_by(window(), "a", 100);
    assert_eq!(w.step(Event::Select(pane("a")), 200), vec![Effect::Verify { after_ms: 0 }]);
}

#[test]
fn an_inactive_window_takes_no_focus_until_it_is_activated() {
    let mut w = window();
    w.step(Event::Deactivated, 100);
    assert!(w.step(Event::Select(pane("b")), 110).is_empty());
    let fx = w.step(Event::Activated, 5000);
    assert_eq!(
        fx,
        vec![Effect::RaisePanes, Effect::FocusPane("b".into()), Effect::Verify { after_ms: VERIFY_AFTER_MS }]
    );
}

#[test]
fn a_selected_pane_that_is_not_live_yet_gives_the_app_the_keys_meanwhile() {
    let mut w = window();
    let fx = w.step(Event::Select(pane("new")), 100);
    assert!(fx.is_empty() || fx == vec![Effect::Verify { after_ms: 0 }], "{fx:?}");
    assert_eq!(w.want, pane("new"));
    // It goes live shortly after: the selection is applied.
    let fx = w.step(Event::PaneLive("new".into()), 400);
    assert_eq!(focus_effects(&fx), vec![&Effect::FocusPane("new".into())]);
}

// ── Clicks ─────────────────────────────────────────────────────────────

#[test]
fn a_click_in_a_page_selects_it() {
    let mut w = window();
    let fx = w.step(Event::PanePointerDown("b".into()), 100);
    assert_eq!(focus_effects(&fx), vec![&Effect::FocusPane("b".into())]);
    assert_eq!(w.want, pane("b"));
}

#[test]
fn a_click_in_the_app_takes_the_keys_back() {
    let mut w = owned_by(window(), "a", 100);
    let fx = w.step(Event::AppPointerDown, 200);
    assert_eq!(focus_effects(&fx), vec![&Effect::FocusApp]);
    assert_eq!(w.want, Owner::App);
}

// ── Focus the platform reports ─────────────────────────────────────────

#[test]
fn a_page_that_takes_the_keyboard_on_its_own_is_put_back() {
    // G6: today the WM_SETFOCUS is swallowed; here it reached Chromium and is corrected after.
    let mut w = owned_by(window(), "a", 100);
    let fx = w.step(Event::FocusGained(at("b")), 500);
    assert_eq!(fx, vec![Effect::FocusPane("a".into()), Effect::Verify { after_ms: VERIFY_AFTER_MS }]);
}

#[test]
fn a_page_refocusing_in_a_loop_is_corrected_at_most_once_per_interval() {
    let mut w = window();
    assert_eq!(focus_effects(&w.step(Event::FocusGained(at("a")), 1000)), vec![&Effect::FocusApp]);
    // Again 30 ms later: put off, not dropped.
    let fx = w.step(Event::FocusGained(at("a")), 1030);
    assert!(focus_effects(&fx).is_empty());
    assert_eq!(fx, vec![Effect::Verify { after_ms: RESTORE_MIN_INTERVAL_MS - 30 }]);
    assert!(w.restore_due);
    // The owed correction runs when the check comes.
    let fx = w.step(Event::Verify(at("a")), 1100);
    assert_eq!(focus_effects(&fx), vec![&Effect::FocusApp]);
    assert!(!w.restore_due);
}

#[test]
fn focus_moving_to_the_app_is_never_reversed() {
    // The user clicked the app; Win32 moved focus before the frontend's selection arrived.
    let mut w = owned_by(window(), "a", 100);
    assert!(w.step(Event::FocusGained(Surface::App), 300).is_empty());
    assert_eq!(w.have, Have::Owned(Surface::App));
}

#[test]
fn the_selected_pane_taking_focus_by_itself_is_accepted() {
    let mut w = window();
    w.step(Event::Select(pane("a")), 100);
    w.step(Event::Verify(at("a")), 150);
    w.step(Event::FocusGained(Surface::App), 200);
    // Chromium hands it back to the selected page.
    assert!(w.step(Event::FocusGained(at("a")), 210).is_empty());
    assert_eq!(w.have, Have::Owned(at("a")));
}

// ── Activation (G3) ────────────────────────────────────────────────────

#[test]
fn activation_restores_the_selection_after_the_platforms_own_handling() {
    let mut w = owned_by(window(), "a", 100);
    w.step(Event::Deactivated, 200);
    // While away, Chromium's own activation handling may report the app first.
    let fx = w.step(Event::Activated, 9000);
    assert_eq!(
        fx,
        vec![Effect::RaisePanes, Effect::FocusPane("a".into()), Effect::Verify { after_ms: VERIFY_AFTER_MS }]
    );
    // The check sees the app (Chromium took it back): ask again.
    let fx = w.step(Event::Verify(Surface::App), 9050);
    assert_eq!(focus_effects(&fx), vec![&Effect::FocusPane("a".into())]);
}

#[test]
fn activation_with_the_selected_pane_hidden_gives_the_app_the_keys() {
    let mut w = owned_by(window(), "a", 100);
    w.step(Event::PaneCollapsed("a".into()), 200);
    w.step(Event::Deactivated, 300);
    let fx = w.step(Event::Activated, 400);
    assert_eq!(focus_effects(&fx), vec![&Effect::FocusApp]);
}

#[test]
fn a_shown_window_reraises_its_panes() {
    // G9: nothing re-raised a pane after the window was hidden and shown.
    let mut w = window();
    assert_eq!(w.step(Event::WindowShown, 100), vec![Effect::RaisePanes]);
}

// ── Panes hidden, shown, closed (G2) ───────────────────────────────────

#[test]
fn hiding_the_focused_pane_gives_the_app_the_keys() {
    let mut w = owned_by(window(), "a", 100);
    let fx = w.step(Event::PaneCollapsed("a".into()), 5000);
    assert_eq!(fx, vec![Effect::FocusApp, Effect::Verify { after_ms: VERIFY_AFTER_MS }]);
}

#[test]
fn hiding_another_pane_changes_nothing() {
    let mut w = owned_by(window(), "a", 100);
    assert!(w.step(Event::PaneCollapsed("b".into()), 5000).is_empty());
}

#[test]
fn showing_a_pane_again_does_not_take_the_keys_by_itself() {
    // #4958: an old request applied on a later layout pass took the keyboard mid-typing.
    let mut w = owned_by(window(), "a", 100);
    w.step(Event::PaneCollapsed("a".into()), 5000);
    w.step(Event::Verify(Surface::App), 5050);
    assert!(focus_effects(&w.step(Event::PaneShown("a".into()), 9000)).is_empty());
    // ...but a fresh selection of it does.
    let fx = w.step(Event::Select(pane("a")), 9100);
    assert_eq!(focus_effects(&fx), vec![&Effect::FocusPane("a".into())]);
}

#[test]
fn closing_the_selected_pane_selects_the_app() {
    let mut w = owned_by(window(), "a", 100);
    let fx = w.step(Event::PaneClosing("a".into()), 5000);
    assert_eq!(focus_effects(&fx), vec![&Effect::FocusApp]);
    assert_eq!(w.want, Owner::App);
    assert!(w.step(Event::PaneGone("a".into()), 5100).is_empty());
    assert!(!w.panes.contains_key("a"));
}

// ── Verify (G7) ────────────────────────────────────────────────────────

#[test]
fn a_request_that_never_arrives_is_retried_then_accepted_with_a_warning() {
    let mut w = window();
    w.step(Event::Select(pane("a")), 100);
    let mut asks = 1;
    let mut last = vec![];
    for i in 1..=MAX_ATTEMPTS as u64 {
        last = w.step(Event::Verify(Surface::App), 100 + i * 50);
        asks += focus_effects(&last).len();
    }
    assert_eq!(asks, MAX_ATTEMPTS as usize);
    assert_eq!(last, vec![Effect::Warn("focus did not reach the requested owner")]);
    assert_eq!(w.have, Have::Owned(Surface::App));
}

#[test]
fn a_check_records_a_drift_without_reversing_it_and_a_drift_to_the_app_is_the_users_choice() {
    let mut w = owned_by(window(), "a", 100);
    assert!(w.step(Event::Verify(Surface::App), 3000).is_empty());
    assert_eq!(w.have, Have::Owned(Surface::App));
    assert_eq!(w.want, Owner::App);
    // So the pane being shown later can't take it back.
    assert!(focus_effects(&w.step(Event::PaneShown("a".into()), 3100)).is_empty());
}

#[test]
fn a_pending_pane_that_is_hidden_before_the_check_falls_back_to_the_app() {
    let mut w = window();
    w.step(Event::Select(pane("a")), 100);
    w.step(Event::PaneCollapsed("a".into()), 120);
    let fx = w.step(Event::Verify(Surface::App), 150);
    assert!(focus_effects(&fx).iter().all(|e| **e == Effect::FocusApp), "{fx:?}");
}

#[test]
fn describe_reads_well() {
    let mut w = window();
    w.step(Event::Select(pane("0123456789abcdef")), 100);
    // The pane isn't live: the app is asked for, the selection is kept.
    assert_eq!(w.describe(), "active want=pane:01234567 have=app");
    w.step(Event::PaneLive("0123456789abcdef".into()), 200);
    assert_eq!(w.describe(), "active want=pane:01234567 have=pending(pane:01234567#0)");
}

// ── Invariants over random sequences (spec §3.4) ───────────────────────

const PANES: [&str; 3] = ["a", "b", "c"];

fn any_pane() -> impl Strategy<Value = String> {
    prop::sample::select(PANES.to_vec()).prop_map(String::from)
}

fn any_surface() -> impl Strategy<Value = Surface> {
    prop_oneof![
        Just(Surface::App),
        Just(Surface::Elsewhere),
        any_pane().prop_map(Surface::Pane),
    ]
}

fn any_event() -> impl Strategy<Value = Event> {
    prop_oneof![
        prop_oneof![Just(Owner::App), any_pane().prop_map(Owner::Pane)].prop_map(Event::Select),
        any_pane().prop_map(Event::PanePointerDown),
        Just(Event::AppPointerDown),
        any_surface().prop_map(Event::FocusGained),
        Just(Event::Activated),
        Just(Event::Deactivated),
        Just(Event::WindowShown),
        any_pane().prop_map(Event::PaneLive),
        any_pane().prop_map(Event::PaneShown),
        any_pane().prop_map(Event::PaneCollapsed),
        any_pane().prop_map(Event::PaneClosing),
        any_pane().prop_map(Event::PaneGone),
        any_surface().prop_map(Event::Verify),
    ]
}

proptest! {
    #[test]
    fn invariants_hold_after_every_step(steps in prop::collection::vec((any_event(), 0u64..400), 1..80)) {
        let mut w = WindowInput::default();
        let mut now = 0u64;
        for (event, gap) in steps {
            now += gap;
            let before = w.clone();
            let fx = w.step(event.clone(), now);
            let focus = focus_effects(&fx);

            // I3: at most one focus effect per step.
            prop_assert!(focus.len() <= 1, "{event:?} -> {fx:?}");
            // I2: every focus effect is checked.
            if !focus.is_empty() {
                prop_assert!(fx.iter().any(|e| matches!(e, Effect::Verify { .. })), "{event:?} -> {fx:?}");
            }
            // I4: an inactive window produces no focus effects.
            if !w.active {
                prop_assert!(focus.is_empty(), "{event:?} -> {fx:?}");
            }
            // I1: a request names the app or a live pane.
            if let Have::Pending { target, .. } = &w.have {
                prop_assert_eq!(w.usable(target), target.clone(), "{:?} -> {:?}", event, w.have);
            }
            // I6: every focus effect names the app or the selection.
            for e in &focus {
                if let Effect::FocusPane(b) = e {
                    prop_assert_eq!(&w.want, &Owner::Pane(b.clone()), "{:?} -> {:?}", event, fx);
                }
            }
            // I7: a pane becoming live or shown never applies an old selection.
            if matches!(event, Event::PaneLive(_) | Event::PaneShown(_))
                && now.saturating_sub(before.want_at_ms) > PARKED_WANT_TTL_MS
            {
                prop_assert!(focus.is_empty(), "{event:?} -> {fx:?}");
            }
            // A pending request is always tied to a check (a deadline): asking
            // again without one would leave it pending forever.
            if matches!(w.have, Have::Pending { .. }) && w.have != before.have {
                prop_assert!(fx.iter().any(|e| matches!(e, Effect::Verify { .. })), "{event:?} -> {fx:?}");
            }
        }
    }

    #[test]
    fn a_page_is_corrected_at_most_once_per_interval(
        times in prop::collection::vec(0u64..60, 1..40),
    ) {
        let mut w = window();
        let mut now = 1000u64;
        let mut corrections: Vec<u64> = vec![];
        for gap in times {
            now += gap;
            let fx = w.step(Event::FocusGained(at("b")), now);
            if !focus_effects(&fx).is_empty() {
                corrections.push(now);
            }
        }
        for pair in corrections.windows(2) {
            prop_assert!(pair[1] - pair[0] >= RESTORE_MIN_INTERVAL_MS, "{corrections:?}");
        }
    }
}
