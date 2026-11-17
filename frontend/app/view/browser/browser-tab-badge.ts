// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabBadge } from "@/app/element/pane-tab-model";
import type { BrowserIdentity } from "@/app/util/browser-identity";
import { slotRoleColor } from "@/app/block/pane-color-scheme";
import { isLightThemeActive } from "@/app/block/pane-identity";
import { slotOf } from "@/app/theme/palette";
import { profileById } from "./browser-profiles";

/**
 * The colour a profile's mark takes where it is drawn straight on the app's chrome (a tab's dot, the toolbar's
 * Profile button): the profile's own colour, and for Black and White the neutral's line tone for the theme. A
 * black mark cannot be seen on dark chrome, nor a white one on light chrome. Reactive to the theme.
 */
export function profileMarkColor(color: string | undefined): string | undefined {
    const slot = slotOf(color);
    return slot?.neutral ? slotRoleColor(slot, isLightThemeActive(), "identity") : color;
}

/**
 * The mark a tab of this identity carries on its icon: a named profile's colour (the accent where it
 * has none), a muted grey for Incognito, nothing for Personal
 * (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §4). A Black or White profile's dot is drawn in
 * the neutral's line tone (`profileMarkColor`).
 */
export function identityBadge(id: BrowserIdentity): PaneTabBadge | undefined {
    if (id.kind === "incognito") return { color: "var(--secondary-text-color)", label: "Incognito" };
    if (id.kind === "profile") {
        const profile = profileById(id.id);
        return { color: profileMarkColor(profile?.color) ?? "var(--accent-color)", label: profile?.name ?? "Browser profile" };
    }
    return undefined;
}
