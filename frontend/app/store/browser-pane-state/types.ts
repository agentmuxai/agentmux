// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the browser-pane-state reducer (slice #9 in
 * docs/specs/frontend-reducer-architecture-2026-05-03.md and the
 * roadmap at docs/specs/browser-pane-reducer-roadmap.md).
 *
 * **Phases 3a + 3b + 3c + 3e (title) + 3d.** This reducer owns seven
 * cells today:
 *   - `closed` — terminal flag set by `dispose()`. All post-close
 *     commands are no-ops.
 *   - `loading` — page-load-in-flight flag. Mutually exclusive with
 *     `error` (one or the other; never both).
 *   - `error` — page-load failure message. Mutually exclusive with
 *     `loading`.
 *   - `canGoBack` / `canGoForward` — history navigability flags
 *     mirrored from CEF's `on_loading_state_change_pane` events
 *     (forwarded as `browser-pane-nav-state` with `url_only=false`).
 *   - `title` — page title. Falls back to `"Browser"` when the host
 *     emits empty/whitespace (mirrors the per-catalog rule that the
 *     view should never display a blank pane title).
 *   - `url` — committed/loading URL (NOT the address-bar's
 *     unsubmitted typing buffer — that stays in the view). The
 *     **danger cell** that broke PR #737 — six downstream consumers
 *     (reload's clear+restore, view-mount diag, initial addressBar,
 *     the sync createEffect that syncs urlAtom→addressBar, onMount's
 *     createPane, the placeholder `<Show>` gate); migration must
 *     preserve every consumer's semantics verbatim.
 *
 * Cells not yet migrated: `faviconUrl` (will be a derived projection
 * of `url`'s origin in a follow-up PR) plus the address-bar typing
 * buffer in the view. The slot store + `recordDispatch` audit lands
 * in Phase 4.
 *
 * Note: Phase 6 of the roadmap is when host-side title-change events
 * (`browser-pane-title-change`) actually start emitting. Until then,
 * the reducer's `title` cell stays at its initial fallback.
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
    /** Whether the embedded browser has back-history. Mirror of
     *  CEF's `can_go_back` from `on_loading_state_change_pane`,
     *  delivered to the renderer via the `browser-pane-nav-state`
     *  event with `url_only=false`. CEF is the source of truth; the
     *  reducer only persists the latest mirrored value. */
    canGoBack: boolean;
    /** Whether the embedded browser has forward-history. Mirror of
     *  CEF's `can_go_forward`, same delivery + ownership notes as
     *  `canGoBack`. */
    canGoForward: boolean;
    /** Page title. Always non-empty — empty/whitespace input from
     *  `TitleChanged` is folded to `TITLE_FALLBACK` so the view
     *  never has to know about the empty case. */
    title: string;
    /** Committed/loading URL. Distinct from the address-bar `<input>`
     *  typing buffer (which stays in `browser-view.tsx` per the
     *  catalog — DOM is the source of truth for live-typing UX).
     *  Initial `""` matches the prior signal initial value. Set by
     *  `Navigate` (intent-bearing user/programmatic nav), cleared
     *  transiently by `UrlCleared` (reload's force-reload pattern),
     *  and superseded by `UrlConfirmed` (host's `browser-pane-nav-state`
     *  reporting the post-redirect final URL). */
    url: string;
}

/** The string the reducer projects when `TitleChanged` arrives with
 *  empty or whitespace-only content. The view binds to `state.title`
 *  directly, so the fallback is enforced at write time, not at read
 *  time. Matches the catalog's "falls back to 'Browser' when empty"
 *  rule (`docs/specs/browser-pane-state-catalog.md` row 1.titleAtom). */
export const TITLE_FALLBACK = "Browser";

export const initialState = (): BrowserPaneState => ({
    closed: false,
    loading: false,
    error: null,
    canGoBack: false,
    canGoForward: false,
    title: TITLE_FALLBACK,
    url: "",
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
     * Co-update of the history-navigability flags. Either field may
     * be omitted — the host emits whichever values its
     * `on_loading_state_change_pane` hook gave us, and the reducer
     * leaves an undefined field alone. Identical-to-state values
     * yield no event (idempotent). After dispose, a no-op.
     */
    | {
          type: "HistoryUpdated";
          canGoBack?: boolean;
          canGoForward?: boolean;
      }
    /**
     * Host emitted a title change for this pane. The reducer folds
     * empty/whitespace input to `TITLE_FALLBACK` so the view never
     * sees a blank title. Idempotent — same title yields no event.
     * After dispose, a no-op.
     */
    | { type: "TitleChanged"; title: string }
    /**
     * Host's `browser-pane-nav-state` reported the post-redirect
     * confirmed URL. Updates `state.url` only; does NOT touch
     * loading/error (those transition via `LoadFinished`/`LoadFailed`
     * in the same event handler). Idempotent on identical values.
     * After dispose, a no-op.
     */
    | { type: "UrlConfirmed"; url: string }
    /**
     * Transient URL clear used by `reload()` to force an iframe
     * reload via clear-then-restore across one frame. Sets
     * `state.url = ""` without touching loading/error/title.
     * Idempotent (already-empty url yields no event). After dispose,
     * a no-op.
     */
    | { type: "UrlCleared" }
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
    | {
          type: "history-updated";
          canGoBack: boolean;
          canGoForward: boolean;
      }
    | { type: "title-changed"; title: string }
    | { type: "url-confirmed"; url: string }
    | { type: "url-cleared" }
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
