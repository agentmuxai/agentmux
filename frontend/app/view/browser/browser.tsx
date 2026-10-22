// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// Browser module barrel — its native pane tab manifest. Its code loads with
// the first Browser pane (browser-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { IDENTITY_META_KEY, INCOGNITO_ICON, parseIdentity } from "./browser-identity";
import { profileById } from "./browser-profiles";
import { readSleep, SLEEP_META_KEY } from "./hibernation";

/** The browser as a native pane tab (Pane Tab contract Phase 2c). */
export const browserPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "browser",
    label: "Browser",
    icon: "globe",
    defaultHue: 60,
    // Keep-alive per the repo owner's decision (SPEC_PANE_TAB_CONTRACT_V1 §5):
    // remounting reloads the page. Its page is a native surface, collapsed
    // whenever the tab isn't visible.
    capabilities: { lifecycle: "keepAlive", nativeSurface: true, noPadding: true },
    // An Incognito or named-profile tab says so in the tab strip, in place of
    // the site's icon; Personal tabs keep theirs
    // (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §4).
    tab: {
        icon: (ctx) => {
            // A hibernated pane shows a moon in its tab until it wakes
            // (SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md, the card section).
            if (readSleep(ctx.meta?.[SLEEP_META_KEY])) return { kind: "fa", name: "moon" };
            const id = parseIdentity(ctx.meta?.[IDENTITY_META_KEY]);
            if (id.kind === "incognito") return { kind: "fa", name: INCOGNITO_ICON };
            if (id.kind === "profile") return { kind: "fa", name: "circle-user", color: profileById(id.id)?.color };
            return undefined;
        },
    },
    members: ["liveTitle", "liveFavicon", "contextMenu", "focus"],
    load: () => import("./browser-instance"),
});
