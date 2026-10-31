// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabBadge } from "@/app/element/pane-tab-model";
import type { BrowserIdentity } from "./browser-identity";
import { profileById } from "./browser-profiles";

/**
 * The mark a tab of this identity carries on its icon: a named profile's colour (the accent where it
 * has none), a muted grey for Incognito, nothing for Personal
 * (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §4).
 */
export function identityBadge(id: BrowserIdentity): PaneTabBadge | undefined {
    if (id.kind === "incognito") return { color: "var(--secondary-text-color)", label: "Incognito" };
    if (id.kind === "profile") {
        const profile = profileById(id.id);
        return { color: profile?.color ?? "var(--accent-color)", label: profile?.name ?? "Browser profile" };
    }
    return undefined;
}
