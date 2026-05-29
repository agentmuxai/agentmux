// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Shared "a pane reflow animation is in flight" signal.
//
// Native browser panes are real child windows (HWNDs) that CSS cannot move,
// so they can't ride the `.tile-node` / `.block-content` CSS transitions the
// way DOM panes do. Instead, while a reflow animation is playing, the tiling
// layout pings `notifyPaneReflow()` (on any node geometry change that isn't a
// resize drag), and each browser pane (`browser-view.tsx`) re-samples its
// placeholder's live `getBoundingClientRect()` every frame and forwards it via
// `browser_pane_resize` → `SetWindowPos`. The native window therefore tracks
// exactly what its CSS-animating DOM placeholder is doing — one clock, no drift.
//
// See docs/specs/SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md.

import { createSignal } from "solid-js";

// A little longer than the 150ms layout animation so the per-frame sampling
// covers the easing tail and any rAF/scheduler slack before settling.
const REFLOW_WINDOW_MS = 220;

// Wall-clock instant (performance.now() ms) until which a reflow is considered
// active. Stored in a signal so consumers can reactively start their sampling
// loop the moment a reflow begins.
const [animatingUntil, setAnimatingUntil] = createSignal(0);

/** Called by the layout when a pane's geometry changes during an animation. */
export function notifyPaneReflow(): void {
    setAnimatingUntil(performance.now() + REFLOW_WINDOW_MS);
}

/**
 * True while a reflow is in flight. Reads the `animatingUntil` signal (so a
 * `createEffect` calling this re-runs when a reflow begins) and compares it to
 * the current time (so a per-frame loop calling this stops once the window
 * elapses).
 */
export function paneReflowActive(): boolean {
    return performance.now() < animatingUntil();
}
