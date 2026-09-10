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
 * 180ms originally, which was too short: an ordinary reposition-and-release
 * read as a dock request. Raised to 500ms by
 * `docs/specs/SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md` to sit inside the
 * 400-700ms band comparable hover-intent interactions use, and to match
 * `SPRING_SWITCH_MS` (frontend/app/tab/tabbar-dnd.ts).
 *
 * Lowered to 300ms after using it: 500ms was noticeably sluggish in the hand,
 * and the argument for it was desk research, not experience. That is a
 * deliberate step below the published band — the band describes *passive*
 * hover, where the user has not yet declared intent, whereas a floater drag is
 * already a committed gesture and only the destination is in question.
 *
 * 300ms is not a partial return to the 180ms bug. That failure was mostly
 * mechanical rather than numeric: dwell was inferred from the ABSENCE of hover
 * events, so a pause of any kind armed a redock even after the velocity gate
 * had rejected the entry. Dwell is now measured from confirmed samples (§5.1),
 * so 300ms of genuine stillness is a real signal in a way 180ms never was.
 */
export const REDOCK_DWELL_MS = 300;

/** CSS-px/s above which cursor motion cancels the dwell clock. */
export const REDOCK_VELOCITY_PX_PER_S = 400;
