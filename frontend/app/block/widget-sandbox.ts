// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The `sandbox` attribute of a sandboxed widget's frame
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.1).
 *
 * Only what a widget needs to run: scripts, form controls it handles itself
 * (the CSP's `form-action 'none'` stops a submission) and downloads of what it
 * makes. Everything else stays off, each for a reason:
 * - no `allow-same-origin`: the page keeps an opaque origin;
 * - no `allow-top-navigation*`: it can't send the app's window anywhere;
 * - no `allow-popups*`: it can't open a window of its own;
 * - no `allow-modals`: `alert`, `confirm` and `prompt` would put the widget's
 *   words in a dialog that looks like the app's.
 */
export const WIDGET_FRAME_SANDBOX = "allow-scripts allow-forms allow-downloads";
