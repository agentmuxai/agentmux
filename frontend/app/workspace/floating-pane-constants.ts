// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared redock dwell/velocity constants consumed by both the floater drag
 * handler (floating-pane-workspace.tsx) and the main-window ghost listener
 * (app-init.ts). Single source of truth so both sides gate at the same
 * threshold.
 */

/** Milliseconds the cursor must hover over a target before redock arms. */
export const REDOCK_DWELL_MS = 180;

/** CSS-px/s above which cursor motion cancels the dwell clock. */
export const REDOCK_VELOCITY_PX_PER_S = 400;
