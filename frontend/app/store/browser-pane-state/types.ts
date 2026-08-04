// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the browser-pane-state reducer (multi-tab shape).
 *
 * Each pane holds an ordered list of `BrowserTab` records; pane-level state
 * retains only `closed` (terminal flag — once true, all commands are no-ops).
 * Backend-driven commands carry `source: "backend"` to suppress the
 * navigate-event echo loop.
 */

/** A single browsing context within a Browser pane. */
export interface BrowserTab {
    /** Stable per-pane uuid. Survives reorders. */
    id: string;
    /** Last loaded URL. Initial value `""` matches the prior
     *  pane-level signal initial value. */
    url: string;
    /** Page title. Always non-empty — empty/whitespace input from
     *  `TabTitleChanged` (or the legacy `TitleChanged`) is folded to
     *  `TITLE_FALLBACK` so the view never has to know about the empty
     *  case. */
    title: string;
    /** Page favicon URL. Initially derived from `url`'s origin via
     *  `${origin}/favicon.ico`; overridden by a real URL when CEF
     *  fires `on_favicon_urlchange` (commands `FaviconUrlsReceived`
     *  on the active tab, or `TabFaviconChanged` keyed by id). Empty
     *  string when `url` is empty or unparseable; the view falls back
     *  to the globe icon in that case. */
    faviconUrl: string;
    /** Page load in flight. Set by `Navigate` / `TabUrlChanged` (with
     *  non-backend source) / `LoadStarted`, cleared by `LoadFinished`
     *  / `LoadFailed` / `TabLoadingChanged`. Mutually exclusive with
     *  `error` per Invariant 2. */
    loading: boolean;
    /** Most recent page-load failure for this tab. Cleared on the
     *  next navigation; set by `LoadFailed` / `TabLoadFailed`. */
    error: string | null;
    /** Mirror of CEF's `can_go_back` for this tab. */
    canGoBack: boolean;
    /** Mirror of CEF's `can_go_forward` for this tab. */
    canGoForward: boolean;
    /** True once a real `TabTitleChanged` (or legacy `TitleChanged`)
     *  has applied a non-fallback title — used by `UrlConfirmed`'s
     *  cross-origin guard to avoid overwriting the real title with a
     *  hostname placeholder during the race where CEF's title-change
     *  beats its nav-state event. Cleared on each navigate so each
     *  new page starts fresh. Parallel to `faviconOverridden`. */
    titleOverridden: boolean;
    /** True once a `FaviconUrlsReceived` / `TabFaviconChanged` with
     *  a non-empty URL has been applied — prevents the derived-from-
     *  URL heuristic from overwriting a real favicon. Cleared on
     *  each navigate. */
    faviconOverridden: boolean;
    /** VS Code-style preview slot. All tabs pinned (`isPreview: false`)
     *  in Phase 1; the field exists for Phase 2's middle-click /
     *  background-tab semantics. */
    isPreview: boolean;
    /** True once the backend `browser_pane_tab_create` IPC has
     *  resolved. The view uses this in Phase 1B/1C to decide between
     *  `tab_create` and `tab_show` on activation. Hydrated tabs
     *  start `false`. */
    backendCreated: boolean;
}

/** Entry in the per-pane recently-closed stack. Bounded by
 *  `MAX_RECENTLY_CLOSED` (10). Older entries evicted on overflow. */
export interface ClosedBrowserTab {
    url: string;
    title: string;
    closedAt: number;
}

/** Pane-level reducer state. Per-tab fields live in `tabs[i]`. */
export interface BrowserPaneState {
    /** Terminal flag — `dispose()` sets this. After it flips true,
     *  every subsequent command is a no-op (emits
     *  `post-close-command-dropped` instead of mutating). The model
     *  reads this to gate late IPC handlers. */
    closed: boolean;
    /** Ordered list of tabs. Empty until the view dispatches the
     *  initial `OpenTab`. */
    tabs: BrowserTab[];
    /** Id of the currently active tab, or `null` when `tabs.length === 0`.
     *  Invariant 1: always points at a tab in `tabs[]` or is null. */
    activeTabId: string | null;
    /** Stack of recently-closed tabs, capped at `MAX_RECENTLY_CLOSED`.
     *  Newest entry at the end; `ReopenLastClosed` pops from the end. */
    recentlyClosed: ClosedBrowserTab[];
}

export const MAX_RECENTLY_CLOSED = 10;

/**
 * Derive the favicon URL from a tab URL. Empty string when the URL
 * is empty or unparseable — the view interprets that as "no favicon,
 * show the globe icon" per the catalog. Pure function so it's safe to
 * call inside reducer transitions and from tests directly.
 */
export function deriveFaviconUrl(url: string): string {
    if (url === "") return "";
    try {
        const origin = new URL(url).origin;
        // about:blank, file://, and other schemes that produce an
        // origin of "null" don't have a meaningful favicon — return
        // empty so the view shows the globe.
        if (origin === "null" || origin === "") return "";
        return `${origin}/favicon.ico`;
    } catch {
        return "";
    }
}

/**
 * Hostname-based placeholder for the page title while CEF is loading.
 * Returns the URL's hostname with a leading `www.` stripped (so
 * `https://www.x.com/foo` → `"x.com"`). Empty for unparseable URLs or
 * `null`-origin schemes (about:blank, file://) so the existing
 * `TITLE_FALLBACK` ("Browser") rule takes over.
 */
export function deriveTitlePlaceholder(url: string): string {
    if (url === "") return "";
    try {
        const u = new URL(url);
        if (u.origin === "null" || u.origin === "") return "";
        return u.hostname.replace(/^www\./, "");
    } catch {
        return "";
    }
}

/**
 * True iff `a` and `b` resolve to the same `URL.origin`. Empty strings,
 * unparseable URLs, and `null`-origin schemes (about:blank, file://)
 * compare as same-origin only with themselves.
 */
export function sameOriginUrl(a: string, b: string): boolean {
    if (a === b) return true;
    let originA: string | null = null;
    let originB: string | null = null;
    try { originA = new URL(a).origin; } catch { originA = null; }
    try { originB = new URL(b).origin; } catch { originB = null; }
    if (originA == null || originB == null) return false;
    if (originA === "null" || originB === "null") return false;
    return originA === originB;
}

/** The string projected when a tab's title arrives empty/whitespace.
 *  The view binds to `tab.title` directly, so the fallback is enforced
 *  at write time, not at read time. */
export const TITLE_FALLBACK = "Browser";

export const initialState = (): BrowserPaneState => ({
    closed: false,
    tabs: [],
    activeTabId: null,
    recentlyClosed: [],
});

/**
 * Build a fresh tab record for the given URL. Title / favicon derive
 * from the URL; loading defaults to `true` because the view treats
 * `OpenTab` as the start of a navigation.
 */
export function makeTab(url: string, isPreview = false): BrowserTab {
    const placeholder = deriveTitlePlaceholder(url);
    return {
        id: newTabId(),
        url,
        title: placeholder !== "" ? placeholder : TITLE_FALLBACK,
        faviconUrl: deriveFaviconUrl(url),
        loading: url !== "",
        error: null,
        canGoBack: false,
        canGoForward: false,
        titleOverridden: false,
        faviconOverridden: false,
        isPreview,
        backendCreated: false,
    };
}

/** uuid generator with a deterministic-fallback for older Node test
 *  runners. */
function newTabId(): string {
    if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
        return crypto.randomUUID();
    }
    return `tab-${Math.random().toString(36).slice(2)}-${Date.now()}`;
}

/** Source-tag carried on every dispatch. Backend-driven events use
 *  `"backend"` to suppress the `navigate` event emission and prevent
 *  the echo loop. Defaults to `"frontend"`. */
export type BrowserCommandSource = "frontend" | "backend" | "system" | "hydrate";

/** Hydration shape — input to `HydrateFromMeta`. Carries only the
 *  identity + URL needed to re-create the tab record; everything
 *  else (loading, error, backendCreated, titleOverridden, ...)
 *  resets to its initial value. */
interface HydratedBrowserTab {
    id: string;
    url: string;
    title?: string;
    faviconUrl?: string;
    isPreview?: boolean;
}

export type BrowserPaneCommand =
    /**
     * Append a new tab and (by default) activate it. `mode: "background"`
     * appends without activating — used for Phase 2 middle-click-link
     * semantics. Emits `TabOpened` plus `TabActivated` when the tab
     * becomes active.
     */
    | {
          type: "OpenTab";
          url: string;
          mode?: "foreground" | "background";
          source?: BrowserCommandSource;
      }
    /**
     * Remove the tab. If it was active, activate its right neighbour
     * (or the new last tab when it was the rightmost). Pushes a
     * `ClosedBrowserTab` entry onto the `recentlyClosed` stack.
     * Emits `LastTabClosed` when the pane becomes tabless.
     */
    | { type: "CloseTab"; tabId: string; source?: BrowserCommandSource }
    /**
     * Activate a tab. No-op when the id is unknown or already active.
     */
    | { type: "SwitchTab"; tabId: string; source?: BrowserCommandSource }
    /**
     * Move a tab to `toIndex`. Index is clamped to `[0, tabs.length-1]`.
     * No-op when only one tab exists.
     */
    | {
          type: "ReorderTab";
          tabId: string;
          toIndex: number;
          source?: BrowserCommandSource;
      }
    /** Backend reported a URL change (typically `OnAddressChange`). */
    | {
          type: "TabUrlChanged";
          tabId: string;
          url: string;
          source?: BrowserCommandSource;
      }
    /** Backend reported a title change (`OnTitleChange`). */
    | { type: "TabTitleChanged"; tabId: string; title: string; source?: BrowserCommandSource }
    /** Backend reported a favicon-URL change (`OnFaviconURLChange`). */
    | {
          type: "TabFaviconChanged";
          tabId: string;
          faviconUrl: string;
          source?: BrowserCommandSource;
      }
    /** Backend reported a loading-state change (`OnLoadingStateChange`).
     *  Updates loading, canGoBack, and canGoForward atomically. */
    | {
          type: "TabLoadingChanged";
          tabId: string;
          loading: boolean;
          canGoBack: boolean;
          canGoForward: boolean;
          source?: BrowserCommandSource;
      }
    /** Backend reported a page-load failure (`OnLoadError`). */
    | {
          type: "TabLoadFailed";
          tabId: string;
          error: string;
          source?: BrowserCommandSource;
      }
    /** The view confirmed that `browser_pane_tab_create` resolved. */
    | { type: "TabBackendCreated"; tabId: string; source?: BrowserCommandSource }
    /** Pop the newest entry off `recentlyClosed` and re-open it as a
     *  new tab. No-op when the stack is empty. */
    | { type: "ReopenLastClosed"; source?: BrowserCommandSource }
    /** Bulk-restore from persisted block meta. Each tab starts with
     *  `backendCreated: false`, `loading: false`, `error: null`. */
    | {
          type: "HydrateFromMeta";
          tabs: HydratedBrowserTab[];
          activeTabId: string | null;
          source?: BrowserCommandSource;
      }
    /**
     * The user (or the host's link-click forwarding) initiated a
     * navigation in the active tab. Sets loading=true, clears error.
     * After dispose, a no-op. No-op when `activeTabId === null`.
     */
    | { type: "Navigate"; url: string; source?: BrowserCommandSource }
    /**
     * A load began but the destination URL isn't known yet (e.g.
     * `goBack` / `goForward`). Same shape-effect as `Navigate` for
     * the active tab's loading flag.
     */
    | { type: "LoadStarted" }
    /**
     * Host's `browser-pane-nav-state` event arrived with `url_only=false`,
     * meaning the active tab finished loading.
     */
    | { type: "LoadFinished" }
    /**
     * The active tab's page-load failed. Sets error, clears loading.
     */
    | { type: "LoadFailed"; reason: string }
    /**
     * Co-update of the active tab's history-navigability flags.
     */
    | {
          type: "HistoryUpdated";
          canGoBack?: boolean;
          canGoForward?: boolean;
      }
    /**
     * Host emitted a title change for the active tab. Empty/whitespace
     * folds to `TITLE_FALLBACK`.
     */
    | { type: "TitleChanged"; title: string }
    /**
     * Host's `browser-pane-nav-state` reported the post-redirect
     * confirmed URL for the active tab. Updates active tab's `url`
     * only; does NOT touch loading/error.
     */
    | { type: "UrlConfirmed"; url: string }
    /**
     * Transient URL clear used by `reload()` to force an iframe
     * reload via clear-then-restore across one frame. Sets the
     * active tab's `url = ""`.
     */
    | { type: "UrlCleared" }
    /**
     * The pane HWND captured a click at the Win32 level. Pure event
     * — no state change. Side-effects handled by the slot store's
     * event sink (blur+refocus).
     */
    | { type: "PaneClicked" }
    /**
     * CEF fired `on_favicon_urlchange` for the active tab.
     */
    | { type: "FaviconUrlsReceived"; urls: string[] }
    /**
     * The pane is being torn down. After this command runs, every
     * subsequent command on this state is a no-op. Idempotent.
     */
    | { type: "Disposed" };

export type BrowserPaneEvent =
    | { type: "tab-opened"; tabId: string; url: string; atIndex: number }
    | { type: "tab-closed"; tabId: string; url: string }
    | { type: "tab-activated"; tabId: string }
    | { type: "tab-reordered"; tabId: string; toIndex: number }
    | { type: "tab-url-changed"; tabId: string; url: string }
    | { type: "tab-title-changed"; tabId: string; title: string }
    | { type: "tab-favicon-changed"; tabId: string; faviconUrl: string }
    | {
          type: "tab-loading-changed";
          tabId: string;
          loading: boolean;
          canGoBack: boolean;
          canGoForward: boolean;
      }
    | { type: "tab-load-failed"; tabId: string; error: string }
    | { type: "tab-backend-created"; tabId: string }
    | { type: "last-tab-closed" }
    | { type: "tabs-restored"; tabIds: string[]; activeTabId: string | null }
    // ─── Suppression events (convention §2: "Suppressed/dropped commands
    //     MUST emit a 'negative' event"). Surfaced to the diagnostics
    //     ring so a late IPC or buggy caller is visible in audit rather
    //     than swallowed.
    | { type: "close-suppressed"; tabId: string; reason: "unknown-tab" }
    | { type: "switch-suppressed"; tabId: string; reason: "unknown-tab" }
    | {
          type: "reorder-suppressed";
          tabId: string;
          reason: "unknown-tab" | "noop" | "single-tab";
      }
    | { type: "reopen-empty" }
    | {
          type: "hydrate-suppressed";
          reason: "duplicate-tab-ids" | "empty-tabs-with-active-id";
      }
    /** Active-tab navigate intent. Suppressed when the originating
     *  command carried `source: "backend"` (echo-loop guard). */
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
    | { type: "pane-clicked" }
    | { type: "favicon-urls-received"; url: string }
    | { type: "disposed" }
    /** Invariant fire — emitted instead of mutating state when a
     *  command targets a closed pane. */
    | { type: "post-close-command-dropped"; commandType: string };

export interface ReducerResult {
    state: BrowserPaneState;
    events: BrowserPaneEvent[];
}
