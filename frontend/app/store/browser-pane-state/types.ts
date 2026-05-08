// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * State shape, command (action) union, and event union for the browser
 * pane reducer. See docs/specs/browser-pane-reducer.md for the design
 * rationale (why a reducer at all; why this carve-up of state).
 */

export interface BrowserPaneState {
    /** Stable block id; immutable for the lifetime of this state. */
    readonly blockId: string;

    /** Current URL — loading target or post-redirect committed URL. */
    url: string;

    /** Page <title>; empty defaults to "Browser" when read by viewName. */
    title: string;

    /** Derived favicon URL. Empty → header renders the globe fallback. */
    faviconUrl: string;

    /** True between NavigateRequested / Back / Forward / Reload and the
     *  next NavStateReceived (or LoadError). Mutually exclusive with
     *  `error`. */
    loading: boolean;

    /** Last error message; null when no error. Setting this clears
     *  `loading`; setting `loading=true` clears this. */
    error: string | null;

    /** History gates from CEF's loading-state-change. NavStateReceived
     *  with urlOnly=true does NOT touch these — see invariant #4. */
    canGoBack: boolean;
    canGoForward: boolean;

    /** Lifecycle terminal flag. Once true, all subsequent commands
     *  except the redundant `Disposed` are no-ops. */
    closed: boolean;
}

export type BrowserPaneCommand =
    // Issued by the model's public methods (URL bar submit, history buttons).
    | { type: "NavigateRequested"; url: string }
    | { type: "BackRequested" }
    | { type: "ForwardRequested" }
    | { type: "ReloadRequested" }
    | { type: "Disposed" }
    // Issued by the saga in response to host IPC events.
    | { type: "NavStateReceived"; url: string; canGoBack?: boolean; canGoForward?: boolean; urlOnly: boolean }
    | { type: "TitleChangeReceived"; title: string }
    | { type: "LoadError"; message: string }
    | { type: "Clicked" };

export type BrowserPaneEvent =
    /** Saga: invoke `browser_pane_navigate` IPC with the normalized URL. */
    | { type: "ipc-navigate"; url: string }
    /** Saga: invoke `browser_pane_go_back` IPC. */
    | { type: "ipc-back" }
    /** Saga: invoke `browser_pane_go_forward` IPC. */
    | { type: "ipc-forward" }
    /** Saga: persist URL to block.meta.url so pane restore lands on the latest. */
    | { type: "meta-persist-url"; url: string }
    /** Saga: refocus this block in the layout (click → focus). */
    | { type: "focus-block" }
    /** Saga: stop subscriptions / cleanup (paired with Disposed). */
    | { type: "shutdown" };

export interface ReducerResult {
    state: BrowserPaneState;
    events: BrowserPaneEvent[];
}

export const initialState = (blockId: string): BrowserPaneState => ({
    blockId,
    // url is intentionally empty until NavigateRequested fires post-init.
    // The UI's URL bar reads from this; an empty string is fine for the
    // brief registration window before the constructor dispatches
    // NavigateRequested with the meta.url / DEFAULT_BROWSER_URL.
    url: "",
    title: "Browser",
    faviconUrl: "",
    loading: false,
    error: null,
    canGoBack: false,
    canGoForward: false,
    closed: false,
});
