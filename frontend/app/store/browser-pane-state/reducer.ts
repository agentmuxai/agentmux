// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the browser-pane state slice.
 *
 * **Phase 1A** (#1103) — extends slice #9 to hold an ordered tab list.
 * Per-page fields (`url`, `title`, `loading`, `error`, `canGoBack`,
 * `canGoForward`, `faviconUrl`) move
 * into `BrowserTab` records. Existing commands (`Navigate`, `LoadStarted`,
 * `LoadFinished`, `LoadFailed`, `HistoryUpdated`, `TitleChanged`,
 * `UrlConfirmed`, `UrlCleared`, `FaviconUrlsReceived`) operate on the
 * active tab; when `activeTabId === null` they are defensive no-ops.
 *
 * Invariants enforced:
 *   1. `activeTabId` always points to a tab in `tabs[]` or is null
 *      when `tabs.length === 0`.
 *   2. Tab `id`s are unique within a pane (uuid generator).
 *   3. `recentlyClosed.length <= MAX_RECENTLY_CLOSED` (oldest evicted).
 *   4. Once `closed` flips true, every subsequent command emits
 *      `post-close-command-dropped` and returns the unchanged state.
 *      `Disposed` itself is idempotent.
 *   5. Per-tab `loading` and `error` are mutually exclusive — at most
 *      one of the two is truthy at any time for a given tab.
 *   6. Tab-mutating commands with a non-matching `tabId` are no-ops
 *      (defensive).
 *   7. Backend-source commands (`source: "backend"`) suppress the
 *      `navigate` event emission to prevent the echo loop where
 *      `OnAddressChange` is interpreted as a navigate intent.
 */

import {
    BrowserCommandSource,
    BrowserPaneCommand,
    BrowserPaneEvent,
    BrowserPaneState,
    BrowserTab,
    ClosedBrowserTab,
    MAX_RECENTLY_CLOSED,
    ReducerResult,
    TITLE_FALLBACK,
    deriveFaviconUrl,
    deriveTitlePlaceholder,
    makeTab,
    sameOriginUrl,
} from "./types";

// ─────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────

function findTabIndex(state: BrowserPaneState, tabId: string): number {
    return state.tabs.findIndex((t) => t.id === tabId);
}

function replaceTab(
    state: BrowserPaneState,
    tabId: string,
    patch: (tab: BrowserTab) => BrowserTab,
): BrowserPaneState | null {
    const idx = findTabIndex(state, tabId);
    if (idx < 0) return null;
    const nextTab = patch(state.tabs[idx]);
    if (nextTab === state.tabs[idx]) return state;
    const nextTabs = [
        ...state.tabs.slice(0, idx),
        nextTab,
        ...state.tabs.slice(idx + 1),
    ];
    return { ...state, tabs: nextTabs };
}

/**
 * Applies `patch` to tab `tabId` through `replaceTab`. A null or unknown
 * id, or a patch that returns the tab unchanged, is a no-op with no events
 * (Invariant 6); otherwise `events` describes the patched tab.
 */
function patchTab(
    state: BrowserPaneState,
    tabId: string | null,
    patch: (tab: BrowserTab) => BrowserTab,
    events: (tab: BrowserTab) => BrowserPaneEvent[],
): ReducerResult {
    if (tabId == null) return { state, events: [] };
    const next = replaceTab(state, tabId, patch);
    if (next == null || next === state) return { state, events: [] };
    return { state: next, events: events(next.tabs[findTabIndex(next, tabId)]) };
}

/** Page title from the page itself; blank falls back and clears the override. */
function withTitle(tab: BrowserTab, title: string): BrowserTab {
    const trimmed = title.trim();
    const next = trimmed === "" ? TITLE_FALLBACK : title;
    const overridden = trimmed !== "";
    if (next === tab.title && overridden === tab.titleOverridden) return tab;
    return { ...tab, title: next, titleOverridden: overridden };
}

/** Favicon from CEF, or the derived `/favicon.ico` when CEF reports none. */
function withFavicon(tab: BrowserTab, cefUrl: string | undefined): BrowserTab {
    const next = cefUrl ?? deriveFaviconUrl(tab.url);
    const overridden = cefUrl !== undefined;
    if (next === tab.faviconUrl && overridden === tab.faviconOverridden) return tab;
    return { ...tab, faviconUrl: next, faviconOverridden: overridden };
}

/**
 * The host settled on `url`. A CEF favicon or title survives a same-origin
 * change (and a favicon already for the new origin survives a cross-origin
 * one); otherwise both fall back to values derived from the new URL.
 */
function withConfirmedUrl(tab: BrowserTab, url: string): BrowserTab {
    if (tab.url === url) return tab;
    const originChanged = sameOriginUrl(tab.url, url) === false;
    const faviconAlreadyForNewOrigin = tab.faviconOverridden
        && sameOriginUrl(tab.faviconUrl, url);
    const keepFaviconOverride = tab.faviconOverridden
        && (!originChanged || faviconAlreadyForNewOrigin);
    return {
        ...tab,
        url,
        faviconUrl: keepFaviconOverride ? tab.faviconUrl : deriveFaviconUrl(url),
        faviconOverridden: keepFaviconOverride ? tab.faviconOverridden : false,
        title: originChanged
            ? (deriveTitlePlaceholder(url) || TITLE_FALLBACK)
            : tab.title,
        titleOverridden: originChanged ? false : tab.titleOverridden,
    };
}

function pushRecentlyClosed(
    list: ClosedBrowserTab[],
    entry: ClosedBrowserTab,
): ClosedBrowserTab[] {
    const next = [...list, entry];
    if (next.length > MAX_RECENTLY_CLOSED) {
        return next.slice(next.length - MAX_RECENTLY_CLOSED);
    }
    return next;
}

/**
 * Pick the new active id after `tabId` has been removed from `prevTabs`.
 * Prefer the right neighbour of the closed tab; fall back to the left
 * when closing the rightmost tab; null when no tabs remain.
 */
function pickNextActiveId(
    prevTabs: BrowserTab[],
    closedIndex: number,
): string | null {
    const remaining = prevTabs.length - 1;
    if (remaining <= 0) return null;
    if (closedIndex < remaining) {
        return prevTabs[closedIndex + 1].id;
    }
    return prevTabs[closedIndex - 1].id;
}

/** Returns true if the command's `source` is `"backend"`. Used to
 *  suppress the `navigate` event for backend-driven URL updates. */
function isBackendSource(source: BrowserCommandSource | undefined): boolean {
    return source === "backend";
}

// ─────────────────────────────────────────────────────────────────────
// Reducer
// ─────────────────────────────────────────────────────────────────────

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
        // ─── Tab list management ───────────────────────────────────
        case "OpenTab": {
            const tab = makeTab(command.url);
            const nextTabs = [...state.tabs, tab];
            const background = command.mode === "background";
            const nextActiveId = background && state.activeTabId != null
                ? state.activeTabId
                : tab.id;
            const events: BrowserPaneEvent[] = [
                {
                    type: "tab-opened",
                    tabId: tab.id,
                    url: command.url,
                    atIndex: nextTabs.length - 1,
                },
            ];
            if (nextActiveId === tab.id) {
                events.push({ type: "tab-activated", tabId: tab.id });
            }
            return {
                state: {
                    ...state,
                    tabs: nextTabs,
                    activeTabId: nextActiveId,
                },
                events,
            };
        }

        case "CloseTab": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) {
                return {
                    state,
                    events: [
                        { type: "close-suppressed", tabId: command.tabId, reason: "unknown-tab" },
                    ],
                };
            }
            const tab = state.tabs[idx];
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                ...state.tabs.slice(idx + 1),
            ];
            const wasActive = state.activeTabId === tab.id;
            const nextActiveId = wasActive
                ? pickNextActiveId(state.tabs, idx)
                : state.activeTabId;
            const closedEntry: ClosedBrowserTab = {
                url: tab.url,
                title: tab.title,
                closedAt: Date.now(),
            };
            const nextState: BrowserPaneState = {
                ...state,
                tabs: nextTabs,
                activeTabId: nextActiveId,
                recentlyClosed: pushRecentlyClosed(
                    state.recentlyClosed,
                    closedEntry,
                ),
            };
            const events: BrowserPaneEvent[] = [
                { type: "tab-closed", tabId: tab.id, url: tab.url },
            ];
            if (wasActive && nextActiveId != null) {
                events.push({ type: "tab-activated", tabId: nextActiveId });
            }
            if (nextTabs.length === 0) {
                events.push({ type: "last-tab-closed" });
            }
            return { state: nextState, events };
        }

        case "SwitchTab": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) {
                return {
                    state,
                    events: [
                        { type: "switch-suppressed", tabId: command.tabId, reason: "unknown-tab" },
                    ],
                };
            }
            if (state.activeTabId === command.tabId) {
                // Already active — idempotent. Don't emit anything (no
                // state change, no diagnostic value in a "suppressed"
                // event for the no-op case; calling code does this
                // regularly via Ctrl+Tab when cycling once).
                return { state, events: [] };
            }
            return {
                state: { ...state, activeTabId: command.tabId },
                events: [{ type: "tab-activated", tabId: command.tabId }],
            };
        }

        case "ReorderTab": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) {
                return {
                    state,
                    events: [
                        { type: "reorder-suppressed", tabId: command.tabId, reason: "unknown-tab" },
                    ],
                };
            }
            if (state.tabs.length <= 1) {
                return {
                    state,
                    events: [
                        { type: "reorder-suppressed", tabId: command.tabId, reason: "single-tab" },
                    ],
                };
            }
            const clamped = Math.max(
                0,
                Math.min(state.tabs.length - 1, command.toIndex),
            );
            if (clamped === idx) {
                return {
                    state,
                    events: [
                        { type: "reorder-suppressed", tabId: command.tabId, reason: "noop" },
                    ],
                };
            }
            const next = [...state.tabs];
            const [moved] = next.splice(idx, 1);
            next.splice(clamped, 0, moved);
            return {
                state: { ...state, tabs: next },
                events: [
                    { type: "tab-reordered", tabId: command.tabId, toIndex: clamped },
                ],
            };
        }

        // ─── Per-tab backend-driven updates ────────────────────────
        case "TabUrlChanged":
            // `tab-url-changed` mirrors state; it is not a `navigate` intent,
            // so there's no echo-loop risk regardless of source. That guard
            // lives in `Navigate`.
            return patchTab(
                state,
                command.tabId,
                (tab) => withConfirmedUrl(tab, command.url),
                () => [{ type: "tab-url-changed", tabId: command.tabId, url: command.url }],
            );

        case "TabTitleChanged":
            return patchTab(
                state,
                command.tabId,
                (tab) => withTitle(tab, command.title),
                (tab) => [{ type: "tab-title-changed", tabId: command.tabId, title: tab.title }],
            );

        case "TabFaviconChanged":
            return patchTab(
                state,
                command.tabId,
                (tab) => withFavicon(tab, command.faviconUrl !== "" ? command.faviconUrl : undefined),
                (tab) => [
                    { type: "tab-favicon-changed", tabId: command.tabId, faviconUrl: tab.faviconUrl },
                ],
            );

        case "TabLoadingChanged":
            return patchTab(
                state,
                command.tabId,
                (tab) => {
                    if (
                        tab.loading === command.loading &&
                        tab.canGoBack === command.canGoBack &&
                        tab.canGoForward === command.canGoForward
                    ) {
                        return tab;
                    }
                    // Invariant 5: a fresh attempt clears any prior error;
                    // loading going false keeps the error from TabLoadFailed.
                    return {
                        ...tab,
                        loading: command.loading,
                        canGoBack: command.canGoBack,
                        canGoForward: command.canGoForward,
                        error: command.loading ? null : tab.error,
                    };
                },
                () => [
                    {
                        type: "tab-loading-changed",
                        tabId: command.tabId,
                        loading: command.loading,
                        canGoBack: command.canGoBack,
                        canGoForward: command.canGoForward,
                    },
                ],
            );

        case "TabLoadFailed":
            return patchTab(
                state,
                command.tabId,
                (tab) => ({ ...tab, loading: false, error: command.error }),
                () => [{ type: "tab-load-failed", tabId: command.tabId, error: command.error }],
            );

        case "TabBackendCreated":
            return patchTab(
                state,
                command.tabId,
                (tab) => (tab.backendCreated ? tab : { ...tab, backendCreated: true }),
                () => [{ type: "tab-backend-created", tabId: command.tabId }],
            );

        case "ReopenLastClosed": {
            if (state.recentlyClosed.length === 0) {
                return { state, events: [{ type: "reopen-empty" }] };
            }
            const last = state.recentlyClosed[state.recentlyClosed.length - 1];
            const trimmed = state.recentlyClosed.slice(0, -1);
            // Pop the entry BEFORE delegating to OpenTab so a tab that
            // somehow already exists at that URL just opens fresh.
            const sub = update(
                { ...state, recentlyClosed: trimmed },
                { type: "OpenTab", url: last.url, source: command.source },
            );
            return sub;
        }

        case "HydrateFromMeta": {
            // Invariant 2: tab ids are unique within a pane. Malformed
            // persisted meta with duplicate ids would silently break
            // `findTabIndex` (returns the first match) and make the
            // saga's per-tab IPC routing ambiguous. Detect + reject.
            const seen = new Set<string>();
            for (const t of command.tabs) {
                if (seen.has(t.id)) {
                    return {
                        state,
                        events: [
                            { type: "hydrate-suppressed", reason: "duplicate-tab-ids" },
                        ],
                    };
                }
                seen.add(t.id);
            }
            // Activating into an empty tab list is invalid — the model
            // would project null setters but request a switch.
            if (command.tabs.length === 0 && command.activeTabId != null) {
                return {
                    state,
                    events: [
                        { type: "hydrate-suppressed", reason: "empty-tabs-with-active-id" },
                    ],
                };
            }
            const tabs: BrowserTab[] = command.tabs.map((t) => ({
                id: t.id,
                url: t.url,
                title: t.title ?? (deriveTitlePlaceholder(t.url) || TITLE_FALLBACK),
                faviconUrl: t.faviconUrl ?? deriveFaviconUrl(t.url),
                loading: false,
                error: null,
                canGoBack: false,
                canGoForward: false,
                titleOverridden: false,
                faviconOverridden: false,
                isPreview: t.isPreview ?? false,
                backendCreated: false,
            }));
            const activeStillPresent =
                command.activeTabId != null &&
                tabs.some((t) => t.id === command.activeTabId);
            const activeId = activeStillPresent
                ? command.activeTabId
                : tabs.length > 0
                  ? tabs[0].id
                  : null;
            const nextState: BrowserPaneState = {
                ...state,
                tabs,
                activeTabId: activeId,
            };
            return {
                state: nextState,
                events: [
                    {
                        type: "tabs-restored",
                        tabIds: tabs.map((t) => t.id),
                        activeTabId: activeId,
                    },
                ],
            };
        }

        // ─── Existing commands — operate on the active tab ─────────
        case "Navigate": {
            // Echo-loop guard (convention §6). The `navigate` event is the
            // host-navigation intent (nothing consumes it yet: the nav bar
            // calls `browser_pane_navigate` on the host directly).
            // If the originating command came FROM the host (it shouldn't
            // for Navigate today — user/programmatic only — but the slot
            // store may relay backend-driven URL applies via this command
            // in future plumbing), suppress the IPC-bound event to avoid
            // a host → reducer → host loop.
            const events: BrowserPaneEvent[] = isBackendSource(command.source)
                ? []
                : [{ type: "navigate", url: command.url }];
            // Optimistic header (carried per-tab in Phase 1A).
            const placeholder = deriveTitlePlaceholder(command.url);
            return patchTab(
                state,
                state.activeTabId,
                (tab) => ({
                    ...tab,
                    loading: true,
                    error: null,
                    url: command.url,
                    faviconUrl: deriveFaviconUrl(command.url),
                    faviconOverridden: false,
                    title: placeholder !== "" ? placeholder : TITLE_FALLBACK,
                    titleOverridden: false,
                }),
                () => events,
            );
        }

        case "LoadStarted":
            return patchTab(
                state,
                state.activeTabId,
                (tab) => ({ ...tab, loading: true, error: null }),
                () => [{ type: "load-started" }],
            );

        case "LoadFinished":
            return patchTab(
                state,
                state.activeTabId,
                (tab) =>
                    !tab.loading && tab.error === null
                        ? tab
                        : { ...tab, loading: false, error: null },
                () => [{ type: "load-finished" }],
            );

        case "LoadFailed":
            return patchTab(
                state,
                state.activeTabId,
                (tab) => ({ ...tab, loading: false, error: command.reason }),
                () => [{ type: "load-failed", reason: command.reason }],
            );

        case "HistoryUpdated":
            return patchTab(
                state,
                state.activeTabId,
                (tab) => {
                    const canGoBack = command.canGoBack !== undefined
                        ? command.canGoBack
                        : tab.canGoBack;
                    const canGoForward = command.canGoForward !== undefined
                        ? command.canGoForward
                        : tab.canGoForward;
                    if (canGoBack === tab.canGoBack && canGoForward === tab.canGoForward) {
                        return tab;
                    }
                    return { ...tab, canGoBack, canGoForward };
                },
                (tab) => [
                    {
                        type: "history-updated",
                        canGoBack: tab.canGoBack,
                        canGoForward: tab.canGoForward,
                    },
                ],
            );

        case "TitleChanged":
            return patchTab(
                state,
                state.activeTabId,
                (tab) => withTitle(tab, command.title),
                (tab) => [{ type: "title-changed", title: tab.title }],
            );

        case "UrlConfirmed":
            return patchTab(
                state,
                state.activeTabId,
                (tab) => withConfirmedUrl(tab, command.url),
                () => [{ type: "url-confirmed", url: command.url }],
            );

        case "UrlCleared":
            return patchTab(
                state,
                state.activeTabId,
                (tab) => (tab.url === "" ? tab : { ...tab, url: "", faviconUrl: "" }),
                () => [{ type: "url-cleared" }],
            );

        case "PaneClicked": {
            // Pure event — no state change.
            return { state, events: [{ type: "pane-clicked" }] };
        }

        case "FaviconUrlsReceived":
            return patchTab(
                state,
                state.activeTabId,
                (tab) => withFavicon(tab, command.urls[0]),
                (tab) => [{ type: "favicon-urls-received", url: tab.faviconUrl }],
            );

        case "Disposed": {
            if (state.closed) return { state, events: [] };
            return {
                state: { ...state, closed: true },
                events: [{ type: "disposed" }],
            };
        }
    }
}
