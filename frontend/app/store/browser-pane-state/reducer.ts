// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the browser-pane state slice.
 *
 * **Phase 1A** (`specs/SPEC_BROWSER_PANE_TABS_2026-05-27.md`) — extends
 * slice #9 to hold an ordered tab list. Per-page fields (`url`, `title`,
 * `loading`, `error`, `canGoBack`, `canGoForward`, `faviconUrl`) move
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

function getActiveTab(state: BrowserPaneState): BrowserTab | null {
    if (state.activeTabId == null) return null;
    return state.tabs.find((t) => t.id === state.activeTabId) ?? null;
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
        case "TabUrlChanged": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            if (tab.url === command.url) return { state, events: [] };
            // Same origin-aware favicon preservation as the legacy
            // UrlConfirmed path. The `tab-url-changed` event we emit is
            // a state-mirror notification, NOT a `navigate` intent — so
            // there's no echo-loop risk here regardless of source. The
            // explicit guard lives where intent-bearing commands
            // (`Navigate`) decide whether to emit `navigate` (the saga
            // consumer treats that event as "call browser_pane_tab_navigate").
            const originChanged = sameOriginUrl(tab.url, command.url) === false;
            const faviconAlreadyForNewOrigin = tab.faviconOverridden
                && sameOriginUrl(tab.faviconUrl, command.url);
            const keepFaviconOverride = tab.faviconOverridden
                && (!originChanged || faviconAlreadyForNewOrigin);
            const faviconUrl = keepFaviconOverride
                ? tab.faviconUrl
                : deriveFaviconUrl(command.url);
            const faviconOverridden = keepFaviconOverride
                ? tab.faviconOverridden
                : false;
            const title = originChanged
                ? (deriveTitlePlaceholder(command.url) || TITLE_FALLBACK)
                : tab.title;
            const titleOverridden = originChanged ? false : tab.titleOverridden;
            const nextTab: BrowserTab = {
                ...tab,
                url: command.url,
                faviconUrl,
                faviconOverridden,
                title,
                titleOverridden,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [
                    { type: "tab-url-changed", tabId: command.tabId, url: command.url },
                ],
            };
        }

        case "TabTitleChanged": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            const trimmed = command.title.trim();
            const next = trimmed === "" ? TITLE_FALLBACK : command.title;
            const overridden = trimmed !== "";
            if (next === tab.title && overridden === tab.titleOverridden) {
                return { state, events: [] };
            }
            const nextTab: BrowserTab = {
                ...tab,
                title: next,
                titleOverridden: overridden,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [
                    { type: "tab-title-changed", tabId: command.tabId, title: next },
                ],
            };
        }

        case "TabFaviconChanged": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            const cefUrl = command.faviconUrl;
            const next = cefUrl !== "" ? cefUrl : deriveFaviconUrl(tab.url);
            const overridden = cefUrl !== "";
            if (next === tab.faviconUrl && overridden === tab.faviconOverridden) {
                return { state, events: [] };
            }
            const nextTab: BrowserTab = {
                ...tab,
                faviconUrl: next,
                faviconOverridden: overridden,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [
                    { type: "tab-favicon-changed", tabId: command.tabId, faviconUrl: next },
                ],
            };
        }

        case "TabLoadingChanged": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            if (
                tab.loading === command.loading &&
                tab.canGoBack === command.canGoBack &&
                tab.canGoForward === command.canGoForward
            ) {
                return { state, events: [] };
            }
            // Maintain Invariant 5: loading clears any prior error
            // when it transitions to true (a fresh attempt is in
            // flight); when loading goes false, error is preserved
            // (it stays from a TabLoadFailed dispatch).
            const error = command.loading ? null : tab.error;
            const nextTab: BrowserTab = {
                ...tab,
                loading: command.loading,
                canGoBack: command.canGoBack,
                canGoForward: command.canGoForward,
                error,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [
                    {
                        type: "tab-loading-changed",
                        tabId: command.tabId,
                        loading: command.loading,
                        canGoBack: command.canGoBack,
                        canGoForward: command.canGoForward,
                    },
                ],
            };
        }

        case "TabLoadFailed": {
            const next = replaceTab(state, command.tabId, (t) => ({
                ...t,
                loading: false,
                error: command.error,
            }));
            if (next == null) return { state, events: [] };
            return {
                state: next,
                events: [
                    { type: "tab-load-failed", tabId: command.tabId, error: command.error },
                ],
            };
        }

        case "TabBackendCreated": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            if (tab.backendCreated) return { state, events: [] };
            const nextTab: BrowserTab = { ...tab, backendCreated: true };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [{ type: "tab-backend-created", tabId: command.tabId }],
            };
        }

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
            const active = getActiveTab(state);
            if (active == null) return { state, events: [] };
            // Optimistic header (carried per-tab in Phase 1A).
            const placeholder = deriveTitlePlaceholder(command.url);
            const title = placeholder !== "" ? placeholder : TITLE_FALLBACK;
            const nextActive: BrowserTab = {
                ...active,
                loading: true,
                error: null,
                url: command.url,
                faviconUrl: deriveFaviconUrl(command.url),
                faviconOverridden: false,
                title,
                titleOverridden: false,
            };
            const idx = findTabIndex(state, active.id);
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextActive,
                ...state.tabs.slice(idx + 1),
            ];
            // Echo-loop guard (convention §6). The `navigate` event tells
            // the saga to call `browser_pane_tab_navigate` on the host.
            // If the originating command came FROM the host (it shouldn't
            // for Navigate today — user/programmatic only — but the slot
            // store may relay backend-driven URL applies via this command
            // in future plumbing), suppress the IPC-bound event to avoid
            // a host → reducer → host loop.
            const events: BrowserPaneEvent[] = isBackendSource(command.source)
                ? []
                : [{ type: "navigate", url: command.url }];
            return {
                state: { ...state, tabs: nextTabs },
                events,
            };
        }

        case "LoadStarted": {
            const active = getActiveTab(state);
            if (active == null) return { state, events: [] };
            const idx = findTabIndex(state, active.id);
            const nextActive: BrowserTab = {
                ...active,
                loading: true,
                error: null,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextActive,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [{ type: "load-started" }],
            };
        }

        case "LoadFinished": {
            const active = getActiveTab(state);
            if (active == null) return { state, events: [] };
            if (!active.loading && active.error === null) {
                return { state, events: [] };
            }
            const idx = findTabIndex(state, active.id);
            const nextActive: BrowserTab = {
                ...active,
                loading: false,
                error: null,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextActive,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [{ type: "load-finished" }],
            };
        }

        case "LoadFailed": {
            const active = getActiveTab(state);
            if (active == null) return { state, events: [] };
            const idx = findTabIndex(state, active.id);
            const nextActive: BrowserTab = {
                ...active,
                loading: false,
                error: command.reason,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextActive,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [{ type: "load-failed", reason: command.reason }],
            };
        }

        case "HistoryUpdated": {
            const active = getActiveTab(state);
            if (active == null) return { state, events: [] };
            const nextBack =
                command.canGoBack !== undefined
                    ? command.canGoBack
                    : active.canGoBack;
            const nextForward =
                command.canGoForward !== undefined
                    ? command.canGoForward
                    : active.canGoForward;
            if (
                nextBack === active.canGoBack &&
                nextForward === active.canGoForward
            ) {
                return { state, events: [] };
            }
            const idx = findTabIndex(state, active.id);
            const nextActive: BrowserTab = {
                ...active,
                canGoBack: nextBack,
                canGoForward: nextForward,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextActive,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [
                    {
                        type: "history-updated",
                        canGoBack: nextBack,
                        canGoForward: nextForward,
                    },
                ],
            };
        }

        case "TitleChanged": {
            const active = getActiveTab(state);
            if (active == null) return { state, events: [] };
            const trimmed = command.title.trim();
            const next = trimmed === "" ? TITLE_FALLBACK : command.title;
            const overridden = trimmed !== "";
            if (next === active.title && overridden === active.titleOverridden) {
                return { state, events: [] };
            }
            const idx = findTabIndex(state, active.id);
            const nextActive: BrowserTab = {
                ...active,
                title: next,
                titleOverridden: overridden,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextActive,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [{ type: "title-changed", title: next }],
            };
        }

        case "UrlConfirmed": {
            const active = getActiveTab(state);
            if (active == null) return { state, events: [] };
            if (active.url === command.url) return { state, events: [] };
            const originChanged = sameOriginUrl(active.url, command.url) === false;
            const faviconAlreadyForNewOrigin = active.faviconOverridden
                && sameOriginUrl(active.faviconUrl, command.url);
            const keepOverride = active.faviconOverridden
                && (!originChanged || faviconAlreadyForNewOrigin);
            const faviconUrl = keepOverride
                ? active.faviconUrl
                : deriveFaviconUrl(command.url);
            const faviconOverridden = keepOverride
                ? active.faviconOverridden
                : false;
            const title = originChanged
                ? (deriveTitlePlaceholder(command.url) || TITLE_FALLBACK)
                : active.title;
            const titleOverridden = originChanged ? false : active.titleOverridden;
            const idx = findTabIndex(state, active.id);
            const nextActive: BrowserTab = {
                ...active,
                url: command.url,
                faviconUrl,
                faviconOverridden,
                title,
                titleOverridden,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextActive,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [{ type: "url-confirmed", url: command.url }],
            };
        }

        case "UrlCleared": {
            const active = getActiveTab(state);
            if (active == null) return { state, events: [] };
            if (active.url === "") return { state, events: [] };
            const idx = findTabIndex(state, active.id);
            const nextActive: BrowserTab = {
                ...active,
                url: "",
                faviconUrl: "",
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextActive,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [{ type: "url-cleared" }],
            };
        }

        case "PaneClicked": {
            // Pure event — no state change.
            return { state, events: [{ type: "pane-clicked" }] };
        }

        case "FaviconUrlsReceived": {
            const active = getActiveTab(state);
            if (active == null) return { state, events: [] };
            const cefUrl = command.urls[0];
            const next = cefUrl ?? deriveFaviconUrl(active.url);
            const overridden = cefUrl !== undefined;
            if (next === active.faviconUrl && active.faviconOverridden === overridden) {
                return { state, events: [] };
            }
            const idx = findTabIndex(state, active.id);
            const nextActive: BrowserTab = {
                ...active,
                faviconUrl: next,
                faviconOverridden: overridden,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextActive,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [{ type: "favicon-urls-received", url: next }],
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
