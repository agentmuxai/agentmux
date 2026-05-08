// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the browser-pane state slice (slice #9, Phases 3a +
 * 3b + 3c). See `docs/specs/browser-pane-reducer-roadmap.md` and the
 * conventions doc `frontend-reducer-conventions-2026-05-03.md`. This
 * module is the exact mirror of slice #4's reducer pattern
 * (agent-pane-state) — same `update(state, command) → { state, events }`
 * shape, same idempotency rules, same no-throw policy.
 *
 * Invariants enforced:
 *   1. Once `closed` flips true, every subsequent command emits
 *      `post-close-command-dropped` and returns the unchanged state.
 *      `Disposed` itself is idempotent (second dispatch is a no-op).
 *   2. `loading` and `error` are mutually exclusive — at most one of
 *      the two is truthy at any time.
 *   3. `Navigate` always clears any prior `error` (a fresh attempt
 *      supersedes the prior failure).
 *   4. `LoadFinished` is a no-op if `loading` was already false AND
 *      `error` was already null (nothing to clear; avoids a spurious
 *      load-finished event on a steady-state pane).
 *   5. `HistoryUpdated` is idempotent: if the supplied fields match
 *      the current state, return the unchanged state with no event.
 *      An omitted field leaves its cell alone (the host can emit
 *      partial updates).
 */

import {
    BrowserPaneCommand,
    BrowserPaneState,
    ReducerResult,
} from "./types";

export function update(
    state: BrowserPaneState,
    command: BrowserPaneCommand,
): ReducerResult {
    if (state.closed && command.type !== "Disposed") {
        return {
            state,
            events: [
                {
                    type: "post-close-command-dropped",
                    commandType: command.type,
                },
            ],
        };
    }

    switch (command.type) {
        case "Navigate": {
            return {
                state: { ...state, loading: true, error: null },
                events: [{ type: "navigate", url: command.url }],
            };
        }

        case "LoadStarted": {
            return {
                state: { ...state, loading: true, error: null },
                events: [{ type: "load-started" }],
            };
        }

        case "LoadFinished": {
            if (!state.loading && state.error === null) {
                return { state, events: [] };
            }
            return {
                state: { ...state, loading: false, error: null },
                events: [{ type: "load-finished" }],
            };
        }

        case "LoadFailed": {
            return {
                state: { ...state, loading: false, error: command.reason },
                events: [{ type: "load-failed", reason: command.reason }],
            };
        }

        case "HistoryUpdated": {
            const nextBack =
                command.canGoBack !== undefined
                    ? command.canGoBack
                    : state.canGoBack;
            const nextForward =
                command.canGoForward !== undefined
                    ? command.canGoForward
                    : state.canGoForward;
            if (
                nextBack === state.canGoBack &&
                nextForward === state.canGoForward
            ) {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    canGoBack: nextBack,
                    canGoForward: nextForward,
                },
                events: [
                    {
                        type: "history-updated",
                        canGoBack: nextBack,
                        canGoForward: nextForward,
                    },
                ],
            };
        }

        case "Disposed": {
            if (state.closed) return { state, events: [] };
            return {
                state: { ...state, closed: true },
                events: [{ type: "disposed" }],
            };
        }
    }
}
