// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the browser-pane-state reducer (slice #9 in
 * docs/specs/frontend-reducer-architecture-2026-05-03.md and the
 * roadmap at docs/specs/browser-pane-reducer-roadmap.md).
 *
 * **Phase 3a + 3b only.** This reducer owns three cells today:
 *   - `closed` — terminal flag set by `dispose()`. All post-close
 *     commands are no-ops.
 *   - `loading` — page-load-in-flight flag. Mutually exclusive with
 *     `error` (one or the other; never both).
 *   - `error` — page-load failure message. Mutually exclusive with
 *     `loading`.
 *
 * Other browser-pane state cells (url, title, favicon, canGoBack,
 * canGoForward, plus the address-bar typing buffer that lives in
 * the view) are **not yet migrated** — the roadmap calls out
 * sequential single-cell migrations because cross-cutting moves
 * (PR #737) regressed the typing path. Cells 3c → 3e land in
 * follow-up PRs; the slot store + `recordDispatch` audit lands in
 * Phase 4 once every cell is reducer-backed.
 */

/** Reducer state. Mutually-exclusive invariant on (loading, error). */
export interface BrowserPaneState {
    /** Terminal flag — `dispose()` sets this. After it flips true,
     *  every subsequent command is a no-op. The model checks this
     *  to gate late IPC handlers (a nav-state event arriving after
     *  the user closed the pane shouldn't write into a torn-down
     *  signal). */
    closed: boolean;
    /** Page load in flight. Set by `Navigate`, cleared by
     *  `LoadFinished` and `LoadFailed`. */
    loading: boolean;
    /** Most recent page-load failure. Set by `LoadFailed`, cleared
     *  by `Navigate` (kicking off a fresh attempt) and `LoadFinished`
     *  (success path). Mutually exclusive with `loading` per
     *  Invariant 2. */
    error: string | null;
}

export const initialState = (): BrowserPaneState => ({
    closed: false,
    loading: false,
    error: null,
});

export type BrowserPaneCommand =
    /**
     * The user (or the host's link-click forwarding) initiated a
     * navigation with a known target URL. Sets loading=true, clears
     * error. After dispose, a no-op.
     */
    | { type: "Navigate"; url: string }
    /**
     * A load began but the destination URL isn't known yet (e.g.
     * `goBack` / `goForward` — CEF owns the history, we just fire
     * the IPC and wait for `nav-state` to tell us what we landed on).
     * Same state-shape effect as `Navigate` but expresses different
     * intent for the audit ring. After dispose, a no-op.
     */
    | { type: "LoadStarted" }
    /**
     * Host's `browser-pane-nav-state` event arrived with `url_only=false`,
     * meaning the page finished loading (not just an in-page hash
     * update). Clears loading and error. After dispose, a no-op.
     */
    | { type: "LoadFinished" }
    /**
     * The page-load failed (SSL error, navigation aborted, network
     * down). Sets error, clears loading. After dispose, a no-op.
     */
    | { type: "LoadFailed"; reason: string }
    /**
     * The pane is being torn down. After this command runs, every
     * subsequent command on this state is a no-op. Idempotent —
     * dispatching `Disposed` twice is a no-op the second time.
     */
    | { type: "Disposed" };

export type BrowserPaneEvent =
    | { type: "navigate"; url: string }
    | { type: "load-started" }
    | { type: "load-finished" }
    | { type: "load-failed"; reason: string }
    | { type: "disposed" }
    /** Invariant fire — emitted instead of mutating state when a
     *  command targets a closed pane. Surfaced for diagnostics so
     *  late IPC handlers can be traced rather than failing
     *  silently. */
    | { type: "post-close-command-dropped"; commandType: string };

export interface ReducerResult {
    state: BrowserPaneState;
    events: BrowserPaneEvent[];
}
