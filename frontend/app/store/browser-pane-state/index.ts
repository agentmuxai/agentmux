// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export { update } from "./reducer";
export type {
    BrowserCommandSource,
    BrowserPaneCommand,
    BrowserPaneEvent,
    BrowserPaneState,
    BrowserTab,
    ClosedBrowserTab,
    HydratedBrowserTab,
    ReducerResult,
} from "./types";
export {
    deriveFaviconUrl,
    deriveTitlePlaceholder,
    initialState,
    makeTab,
    MAX_RECENTLY_CLOSED,
    newTabId,
    sameOriginUrl,
    TITLE_FALLBACK,
} from "./types";
