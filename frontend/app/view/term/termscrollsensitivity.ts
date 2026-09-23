// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared resolution of a terminal's wheel-scroll sensitivity multiplier
 * (xterm.js's `scrollSensitivity` option, driven by the `term:scrollsensitivity`
 * setting — SPEC_TERMINAL_SCROLL_SENSITIVITY_SETTING_2026_08_31.md).
 *
 * Every xterm surface in the app must resolve this the same way, or the
 * setting silently means different things in different panes — the same
 * reasoning `termscrollback.ts` (the sibling module this one mirrors) was
 * built for. `termwrap.ts` (initial construction), `termViewModel.ts` (the
 * Terminal pane's live-update path), and `AgentShellSubblock.tsx` (the
 * agent Shell drawer's live-update path) all resolve through this one
 * function.
 *
 * Unlike `term:scrollback`, this setting has no per-block override — it is
 * intentionally global-only (see the spec's §4.1), so there is only ever
 * one source to resolve, not a settings-value-vs-block-override pair.
 */

/** Used when the configured value is absent, non-numeric, zero, negative,
 *  or non-finite — matches xterm.js's own default, so an absent setting
 *  changes nothing. */
export const DEFAULT_TERM_SCROLL_SENSITIVITY = 1;

/** Lower bound applied after the setting is resolved — matches the
 *  minimum `schema/settings.json`'s `term:scrollsensitivity` documents and
 *  the settings UI enforces on entry (`terminal-section.tsx`). */
export const MIN_TERM_SCROLL_SENSITIVITY = 0.1;

/** Upper bound applied after the setting is resolved — matches the
 *  maximum `schema/settings.json` documents. A value above this can only
 *  be reached by hand-editing the raw settings.json (the UI's own number
 *  input refuses to submit past it), so this clamps rather than rejects:
 *  the user's intent was clearly "as fast as possible", which the maximum
 *  represents, not "revert to normal". */
export const MAX_TERM_SCROLL_SENSITIVITY = 10;

/**
 * Resolve the effective scroll-sensitivity multiplier from a raw
 * `term:scrollsensitivity` setting value.
 */
export function resolveTermScrollSensitivity(raw: unknown): number {
    if (typeof raw !== "number" || !Number.isFinite(raw) || raw <= 0) {
        return DEFAULT_TERM_SCROLL_SENSITIVITY;
    }
    return Math.min(Math.max(raw, MIN_TERM_SCROLL_SENSITIVITY), MAX_TERM_SCROLL_SENSITIVITY);
}
