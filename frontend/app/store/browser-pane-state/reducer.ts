// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the browser pane's URL / title / favicon / loading /
 * history state. See docs/specs/browser-pane-reducer.md for the design
 * + invariants.
 *
 * Invariants enforced:
 *   1. `closed` is terminal. Any command other than `Disposed` after
 *      close is a no-op (state passthrough, no events).
 *   2. block_id filtering happens at the saga boundary, NOT in the
 *      reducer — the reducer trusts every command is for its block.
 *   3. `error` and `loading` are mutually exclusive. Setting one
 *      clears the other.
 *   4. NavStateReceived with `urlOnly=true` does NOT update history
 *      gates — those values are stale at on_load_end_pane (kimi's
 *      race finding); the authoritative values come from
 *      on_loading_state_change_pane (urlOnly=false).
 *   5. Favicon is derived from URL origin in the reducer — saga
 *      doesn't know about /favicon.ico.
 *   6. Empty title falls back to "Browser" at write time.
 *   7. NavigateRequested clears favicon + error and sets loading,
 *      but PRESERVES title — avoids "Browser" flash mid-load.
 */

import type {
    BrowserPaneCommand,
    BrowserPaneEvent,
    BrowserPaneState,
    ReducerResult,
} from "./types";

const ALLOWED_AFTER_CLOSED: ReadonlySet<BrowserPaneCommand["type"]> = new Set(["Disposed"]);

/**
 * Derive a favicon URL from the page URL.
 *
 *   - http(s)/etc with a real origin → `${origin}/favicon.ico`
 *   - about:blank, file:, chrome:, malformed → `""` (header globe)
 */
export function deriveFavicon(url: string): string {
    try {
        const origin = new URL(url).origin;
        return origin && origin !== "null" ? `${origin}/favicon.ico` : "";
    } catch {
        return "";
    }
}

/**
 * Normalize a URL bar submission. Mirrors the previous BrowserViewModel
 * logic so `NavigateRequested` can be dispatched directly with raw user
 * input.
 *
 *   - empty / whitespace → empty (caller drops the command)
 *   - has scheme → returned as-is
 *   - looks domain-like → `https://${url}`
 *   - otherwise → google search
 */
export function normalizeUrl(url: string): string {
    const trimmed = url.trim();
    if (!trimmed) return "";
    if (/^https?:\/\//i.test(trimmed) || trimmed.startsWith("about:")) return trimmed;
    if (trimmed.includes(".") && !trimmed.includes(" ")) return `https://${trimmed}`;
    return `https://www.google.com/search?q=${encodeURIComponent(trimmed)}`;
}

export function update(state: BrowserPaneState, cmd: BrowserPaneCommand): ReducerResult {
    if (state.closed && !ALLOWED_AFTER_CLOSED.has(cmd.type)) {
        return { state, events: [] };
    }
    switch (cmd.type) {
        case "NavigateRequested": {
            const normalized = normalizeUrl(cmd.url);
            if (!normalized) return { state, events: [] };
            return {
                state: {
                    ...state,
                    url: normalized,
                    error: null,
                    loading: true,
                    faviconUrl: "",
                    // title intentionally preserved — avoids "Browser"
                    // flash mid-load; the new title arrives via
                    // TitleChangeReceived shortly after the page loads.
                },
                events: [
                    { type: "ipc-navigate", url: normalized },
                    { type: "meta-persist-url", url: normalized },
                ],
            };
        }
        case "NavStateReceived": {
            const next: BrowserPaneState = {
                ...state,
                url: cmd.url,
                faviconUrl: deriveFavicon(cmd.url),
                loading: false,
                error: null,
            };
            if (!cmd.urlOnly) {
                if (cmd.canGoBack !== undefined) next.canGoBack = cmd.canGoBack;
                if (cmd.canGoForward !== undefined) next.canGoForward = cmd.canGoForward;
            }
            return {
                state: next,
                events: [{ type: "meta-persist-url", url: cmd.url }],
            };
        }
        case "TitleChangeReceived":
            return {
                state: { ...state, title: cmd.title || "Browser" },
                events: [],
            };
        case "BackRequested":
            return {
                state: { ...state, loading: true, error: null },
                events: [{ type: "ipc-back" }],
            };
        case "ForwardRequested":
            return {
                state: { ...state, loading: true, error: null },
                events: [{ type: "ipc-forward" }],
            };
        case "ReloadRequested":
            return {
                state: { ...state, loading: true, error: null },
                events: [{ type: "ipc-navigate", url: state.url }],
            };
        case "LoadError":
            return {
                state: { ...state, loading: false, error: cmd.message },
                events: [],
            };
        case "Clicked":
            return { state, events: [{ type: "focus-block" }] };
        case "Disposed":
            return {
                state: { ...state, closed: true },
                events: [{ type: "shutdown" }],
            };
    }
}

// Re-export for convenience so consumers can import everything from
// "@/app/store/browser-pane-state" once we add a barrel.
export type { BrowserPaneCommand, BrowserPaneEvent, BrowserPaneState, ReducerResult } from "./types";
export { initialState } from "./types";
