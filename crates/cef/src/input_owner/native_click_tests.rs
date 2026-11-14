// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A click in a Views pane's page reaches the input owner (#47567): the message hook's decision, with a
//! fake window system, and what the owner does with it.

use std::collections::HashMap;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_POINTERDOWN, WM_RBUTTONDOWN,
};

use super::super::machine::{Effect, Event, Have, Owner, Surface, WindowInput};
use super::*;

/// Windows by `hwnd`, each with its panes and the overlays over it. A click at `lparam` is `(x, y)` in
/// the window's page (the fake's client area is the page's, with no offset).
struct Fake {
    windows: HashMap<isize, String>,
    panes: HashMap<String, Vec<PaneBox>>,
    holes: HashMap<String, Vec<Rect>>,
}

impl Place for Fake {
    fn page_point(&self, hwnd: isize, lparam: isize, _pointer: bool) -> Option<(String, i32, i32)> {
        let (x, y) = words(lparam);
        self.windows.get(&hwnd).map(|w| (w.clone(), x, y))
    }
    fn panes(&self, window: &str) -> Vec<PaneBox> {
        self.panes.get(window).cloned().unwrap_or_default()
    }
    fn holes(&self, window: &str) -> Vec<Rect> {
        self.holes.get(window).cloned().unwrap_or_default()
    }
}

fn at(x: i32, y: i32) -> isize {
    (((y as u16 as u32) << 16) | (x as u16 as u32)) as isize
}

fn pane_box(block: &str, rect: Rect, shown: bool) -> PaneBox {
    PaneBox { block: block.to_string(), rect, shown }
}

/// `main` (hwnd 1) with panes `a` at (100, 100, 400, 300) and `b` at (600, 100, 400, 300); `float` (hwnd
/// 2) with pane `f` at (0, 40, 300, 200).
fn world() -> Fake {
    Fake {
        windows: HashMap::from([(1, "main".to_string()), (2, "float".to_string())]),
        panes: HashMap::from([
            ("main".to_string(), vec![pane_box("a", (100, 100, 400, 300), true), pane_box("b", (600, 100, 400, 300), true)]),
            ("float".to_string(), vec![pane_box("f", (0, 40, 300, 200), true)]),
        ]),
        holes: HashMap::new(),
    }
}

fn clicked(w: &Fake, message: u32, hwnd: isize, x: i32, y: i32) -> Option<(String, String)> {
    clicked_pane(w, message, hwnd, at(x, y))
}

#[test]
fn a_button_going_down_in_a_pane_page_names_that_pane() {
    let w = world();
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 150, 150), Some(("main".into(), "a".into())));
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 700, 350), Some(("main".into(), "b".into())));
    // Every way a pointer goes down: the other buttons, and touch or pen.
    assert_eq!(clicked(&w, WM_RBUTTONDOWN, 1, 150, 150), Some(("main".into(), "a".into())));
    assert_eq!(clicked(&w, WM_MBUTTONDOWN, 1, 150, 150), Some(("main".into(), "a".into())));
    assert_eq!(clicked(&w, WM_POINTERDOWN, 1, 150, 150), Some(("main".into(), "a".into())));
}

#[test]
fn only_a_button_going_down_counts() {
    let w = world();
    assert_eq!(clicked(&w, WM_LBUTTONUP, 1, 150, 150), None);
    assert_eq!(clicked(&w, WM_MOUSEMOVE, 1, 150, 150), None);
}

#[test]
fn a_click_outside_every_pane_is_the_apps() {
    let w = world();
    // Between the panes, above them (the header), left of them.
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 550, 150), None);
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 150, 50), None);
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 50, 150), None);
    // A pane's edge: its first pixel is in, the one past its last is out.
    assert!(clicked(&w, WM_LBUTTONDOWN, 1, 100, 100).is_some());
    assert!(clicked(&w, WM_LBUTTONDOWN, 1, 499, 399).is_some());
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 500, 399), None);
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 499, 400), None);
}

#[test]
fn a_menu_over_a_pane_takes_the_click_where_it_is() {
    let mut w = world();
    w.holes.insert("main".into(), vec![(200, 150, 100, 120)]);
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 250, 200), None, "in the menu");
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 150, 200), Some(("main".into(), "a".into())), "beside it, still the page");
    // A hole with no area, as an overlay that is laid out but empty, cuts nothing.
    w.holes.insert("main".into(), vec![(200, 150, 0, 0)]);
    assert!(clicked(&w, WM_LBUTTONDOWN, 1, 200, 150).is_some());
}

#[test]
fn a_pane_hidden_whole_is_not_clicked() {
    let mut w = world();
    w.panes.get_mut("main").unwrap()[0].shown = false;
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 150, 150), None);
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 700, 150), Some(("main".into(), "b".into())));
}

#[test]
fn a_pane_with_no_area_is_not_clicked() {
    let mut w = world();
    // A background tab of a stack: laid out at 0 by 0.
    w.panes.get_mut("main").unwrap().push(pane_box("tab2", (100, 100, 0, 0), true));
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 100, 100), Some(("main".into(), "a".into())));
}

#[test]
fn a_second_window_and_a_floating_pane_have_their_own_page() {
    let w = world();
    // The same point is pane `a` in main and nothing in the floating window; its own pane is found there.
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 2, 150, 150), Some(("float".into(), "f".into())));
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 2, 150, 20), None);
    // A window the app doesn't know (another program's) is nobody's.
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 9, 150, 150), None);
}

#[test]
fn a_window_with_no_views_panes_reports_nothing() {
    let mut w = world();
    w.panes.insert("main".into(), vec![]);
    assert_eq!(clicked(&w, WM_LBUTTONDOWN, 1, 150, 150), None);
}

#[test]
fn the_owner_gives_the_pane_the_keyboard_for_a_click_it_is_told_of() {
    // The keyboard is with the app, as in the operator's repro: the click in the page reaches nothing today.
    let mut owner = WindowInput::default();
    owner.step(Event::PaneLive("a".into()), 0);
    owner.step(Event::Activated, 0);
    let seq = owner.verify_seq;
    owner.step(Event::Verify { seq, surface: Surface::App }, 50);
    assert_eq!(owner.have, Have::Owned(Surface::App));

    let w = world();
    let (window, block) = clicked(&w, WM_LBUTTONDOWN, 1, 150, 150).expect("the click is in pane a");
    assert_eq!(window, "main");
    let effects = owner.step(Event::PanePointerDown(block), 1000);
    assert!(effects.contains(&Effect::FocusPane("a".into())), "{effects:?}");
    assert_eq!(owner.want, Owner::Pane("a".into()));
}
