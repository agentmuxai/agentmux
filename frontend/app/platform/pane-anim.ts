// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Shared "a pane geometry change just happened — settle native panes" signal.
//
// Native browser panes are real child windows (HWNDs) that CSS cannot move, so
// a layout change doesn't reposition them the way it does DOM panes. On any node
// geometry change that isn't a resize drag the tiling layout pings
// `notifyPaneReflow()`, opening a short window during which each browser pane
// (`browser-view.tsx`) re-samples its placeholder's live
// `getBoundingClientRect()` and forwards it via `browser_pane_resize` →
// `SetWindowPos`, settling the HWND onto the new rect. (The pane reflow CSS
// animation this once tracked frame-by-frame was removed; the placeholder no
// longer animates, so the window just covers rAF/scheduler slack before the
// rect is final.)
//
// See docs/specs/SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md.

import { createSignal } from "solid-js";

// Short settle window after the layout applies the new rect synchronously —
// covers rAF/scheduler slack so the per-frame sampling lands on the final rect.
// Was 220ms when this tracked a CSS pane reflow animation, then 32ms (≈2 rAF
// frames) after the animation was removed and the placeholder rect became final
// immediately. Tuned down further to 4ms for a snappier post-change settle. The
// resample loop in browser-view.tsx is rAF-gated, so this floors at ~1 frame in
// practice regardless of the exact sub-frame value.
const REFLOW_WINDOW_MS = 4;

// Wall-clock instant (performance.now() ms) until which the post-change settle
// window is open. Stored in a signal so consumers can reactively start their
// sampling loop the moment a change begins.
const [animatingUntil, setAnimatingUntil] = createSignal(0);

/** Called by the layout on a pane geometry change (open/close/split/rebalance). */
export function notifyPaneReflow(): void {
    setAnimatingUntil(performance.now() + REFLOW_WINDOW_MS);
}

/**
 * True while the post-change settle window is open. Reads the `animatingUntil`
 * signal (so a `createEffect` calling this re-runs when a change begins) and
 * compares it to the current time (so a per-frame loop calling this stops once
 * the window elapses).
 */
export function paneReflowActive(): boolean {
    return performance.now() < animatingUntil();
}
