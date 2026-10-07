// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Aggregates every section's colocated settings registry into one flat,
// searchable array — see docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md
// §3.2. Plain module-level data, so it exists on import regardless of which
// section is currently mounted (the gap the search feature needs closed),
// and the search over it.

import { literalFirstSearch } from "@/app/util/fuzzysearch";
import type { SettingsIndexEntry } from "./settings-model";
import { APPEARANCE_SETTINGS } from "./sections/appearance-section";
import { WINDOW_SETTINGS } from "./sections/window-panes-section";
import { TERMINAL_SETTINGS } from "./sections/terminal-section";
import { SOUNDS_SETTINGS } from "./sections/sounds-section";
import { NOTIFICATIONS_SETTINGS } from "./sections/notifications-section";
import { RECORDING_SETTINGS } from "./sections/recording-section";
import { DEVICES_SETTINGS } from "./sections/devices-section";
import { ADVANCED_SETTINGS } from "./sections/advanced-section";

export const SETTINGS_INDEX: SettingsIndexEntry[] = [
    ...Object.values(APPEARANCE_SETTINGS),
    ...Object.values(WINDOW_SETTINGS),
    ...Object.values(TERMINAL_SETTINGS),
    ...Object.values(SOUNDS_SETTINGS),
    ...Object.values(NOTIFICATIONS_SETTINGS),
    ...Object.values(RECORDING_SETTINGS),
    ...Object.values(DEVICES_SETTINGS),
    ...Object.values(ADVANCED_SETTINGS),
];

// Fuzzy fallback, used only when no entry contains the query anywhere.
const FUZZY_KEYS = [
    { name: "label", weight: 0.45 },
    { name: "keywords", weight: 0.35 },
    { name: "description", weight: 0.2 },
];

/**
 * The Settings search (§3.5), best first. Literal first, like the My Agents
 * filter: an entry whose label contains the query comes before one that only
 * matches in its curated keywords (the synonyms, §3.4) or description, and an
 * exact label comes first of all. Ties keep `entries` order. Only when no
 * entry contains the query does fuzzy matching step in, for a typo.
 */
export function searchSettings(entries: SettingsIndexEntry[], query: string): SettingsIndexEntry[] {
    return literalFirstSearch(entries, query, (e) => [e.label, ...e.keywords, e.description], { keys: FUZZY_KEYS });
}
