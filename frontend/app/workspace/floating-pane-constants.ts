// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared redock dwell/velocity constants consumed by both the floater drag
 * handler (floating-pane-workspace.tsx) and the main-window ghost listener
 * (app-init.ts). Single source of truth so both sides gate at the same
 * threshold.
 */

/**
 * Milliseconds the cursor must hover over a target before redock arms.
 *
 * Matches `SPRING_SWITCH_MS` (frontend/app/tab/tabbar-dnd.ts) deliberately:
 * a redock destroys the floating window and grafts the pane into the layout
 * tree, which is strictly less reversible than switching the visible tab, so
 * it does not get a shorter fuse. This was 180ms until
 * `docs/specs/SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md` — below the
 * 400-700ms band every comparable hover-intent interaction uses, and low
 * enough that an ordinary reposition-and-release read as a dock request.
 */
export const REDOCK_DWELL_MS = 500;

/** CSS-px/s above which cursor motion cancels the dwell clock. */
export const REDOCK_VELOCITY_PX_PER_S = 400;
