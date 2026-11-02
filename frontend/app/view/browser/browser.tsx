// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// Browser module barrel — its native pane tab manifest. Its code loads with
// the first Browser pane (browser-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { IDENTITY_META_KEY, parseIdentity } from "@/app/util/browser-identity";
import { identityBadge } from "./browser-tab-badge";
import { readSleep, SLEEP_META_KEY } from "./hibernation";

/** The browser as a native pane tab (Pane Tab contract Phase 2c). */
export const browserPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "browser",
    label: "Browser",
    icon: "globe",
    defaultColor: "lime",
    // Keep-alive per the repo owner's decision (SPEC_PANE_TAB_CONTRACT_V1 §5):
    // remounting reloads the page. Its page is a native surface, collapsed
    // whenever the tab isn't visible.
    capabilities: { lifecycle: "keepAlive", nativeSurface: true, noPadding: true },
    // Every tab keeps the site's icon; an Incognito or named-profile tab adds a small mark at its
    // bottom-right, and a Personal tab has none
    // (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §4).
    tab: {
        // A hibernated pane shows a moon in its tab until it wakes
        // (SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md, the card section).
        icon: (ctx) => (readSleep(ctx.meta?.[SLEEP_META_KEY]) ? { kind: "fa", name: "moon" } : undefined),
        badge: (ctx) => (readSleep(ctx.meta?.[SLEEP_META_KEY]) ? undefined : identityBadge(parseIdentity(ctx.meta?.[IDENTITY_META_KEY]))),
    },
    members: ["liveTitle", "liveFavicon", "contextMenu", "focus"],
    load: () => import("./browser-instance"),
});
