// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The panel's keyboard: Escape minimizes, O/S/P/G pick the scope, Enter allows or denies. */

import { usePanelKeys, type PanelKeyContext } from "./use-panel-keys";
import type { DecisionOutcome } from "./decision-panel-model";
import type { DecisionActions } from "./decision-panel-actions";
import type { DecisionState } from "./decision-panel-state";

const SCOPE_KEYS: Record<string, DecisionOutcome["scope"]> = { o: "once", s: "session", p: "project", g: "global" };

/** Installs the key handling (one effect). */
export function useDecisionKeys(s: DecisionState, d: DecisionActions) {
    // Keys reach this only from this panel's own pane (usePanelKeys below):
    // otherwise a prompt in pane A would react to Esc/Enter typed in pane B,
    // and prompts open in several panes would all fire on one keystroke
    // (#556).
    const handleKey = (e: KeyboardEvent, { inPanel, editable }: PanelKeyContext) => {
        const inFeedback = inPanel && (e.target as HTMLElement | null)?.tagName === "TEXTAREA";

        if (e.key === "Escape") {
            // Esc minimizes ONLY when focus is non-editable or in the
            // panel itself: Esc in the composer textarea (or any editable
            // in the pane) needs to keep its normal meaning — dismiss
            // autocomplete, cancel slash picker, etc.
            if (!editable || inPanel) {
                e.preventDefault();
                e.stopPropagation();
                s.setMinimized(true);
                s.props.onDefer?.();
            }
            return;
        }
        // Letter-keyed scope: only when not typing into any editable
        // (composer, feedback textarea, etc.). Composer keystrokes
        // would otherwise be hijacked.
        if (!editable && e.key.length === 1) {
            const sc = SCOPE_KEYS[e.key.toLowerCase()];
            if (sc) {
                e.preventDefault();
                s.setScope(sc);
                return;
            }
        }
        if (e.key === "Enter") {
            // Inside the deny feedback textarea: Enter inserts a newline
            // (default browser behaviour); Shift+Enter sends the denial.
            if (inFeedback) {
                if (e.shiftKey) {
                    e.preventDefault();
                    d.dispatchDeny();
                }
                return;
            }
            // Outside any editable element OR inside the panel itself:
            // Enter = Allow (or arm high-risk), Shift+Enter = enter deny mode.
            if (!editable || inPanel) {
                if (s.denyMode()) {
                    e.preventDefault();
                    d.dispatchDeny();
                } else if (e.shiftKey) {
                    e.preventDefault();
                    s.setDenyMode(true);
                } else {
                    e.preventDefault();
                    d.dispatchAllow(e);
                }
            }
            // If editable and not in panel (composer): let the composer
            // handle Enter. The user must click the panel to act on it.
        }
    };

    // Decision shortcuts work while focus is elsewhere (e.g. the composer).
    usePanelKeys(() => s.rootEl, () => !!s.request(), handleKey);
}
