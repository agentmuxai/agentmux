// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Aggregates every section's colocated settings registry into one flat,
// searchable array — see docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md
// §3.2. Plain module-level data, so it exists on import regardless of which
// section is currently mounted (the gap the search feature needs closed),
// and the search over it.

import { literalFirstSearch } from "@/app/util/fuzzysearch";
import { SETTINGS_SECTION_LABELS, type SettingsIndexEntry } from "./settings-model";
import { APPEARANCE_SETTINGS } from "./sections/appearance-section";
import { ANIMATIONS_SETTINGS } from "./sections/animations-section";
import { WINDOW_SETTINGS } from "./sections/window-panes-section";
import { BROWSER_SETTINGS } from "./sections/browser-section";
import { TERMINAL_SETTINGS } from "./sections/terminal-section";
import { SOUNDS_SETTINGS } from "./sections/sounds-section";
import { NOTIFICATIONS_SETTINGS } from "./sections/notifications-section";
import { RECORDING_SETTINGS } from "./sections/recording-section";
import { DEVICES_SETTINGS } from "./sections/devices-section";
import { WIDGETS_SETTINGS } from "./sections/widgets-section";
import { WIDGET_BAR_SETTINGS } from "./sections/widget-bar-settings";
import { AGENTS_SETTINGS } from "./sections/agents-section";
import { ADVANCED_SETTINGS } from "./sections/advanced-section";
import { SUPERVISOR_SETTINGS } from "./sections/supervisor-link";

export const SETTINGS_INDEX: SettingsIndexEntry[] = [
    ...Object.values(APPEARANCE_SETTINGS),
    ...Object.values(ANIMATIONS_SETTINGS),
    ...Object.values(WINDOW_SETTINGS),
    ...Object.values(BROWSER_SETTINGS),
    ...Object.values(TERMINAL_SETTINGS),
    ...Object.values(SOUNDS_SETTINGS),
    ...Object.values(NOTIFICATIONS_SETTINGS),
    ...Object.values(RECORDING_SETTINGS),
    ...Object.values(DEVICES_SETTINGS),
    ...Object.values(WIDGET_BAR_SETTINGS),
    ...Object.values(WIDGETS_SETTINGS),
    ...Object.values(AGENTS_SETTINGS),
    ...Object.values(SUPERVISOR_SETTINGS),
    ...Object.values(ADVANCED_SETTINGS),
];

// Fuzzy fallback, used only when no entry contains the query anywhere.
const FUZZY_KEYS = [
    { name: "label", weight: 0.45 },
    { name: "keywords", weight: 0.35 },
    { name: "description", weight: 0.2 },
];

/** Where a setting sits in the pane, outermost first, ending in its own name:
 *  ["Sounds", "Tool-call tones", "Volume"]. Two settings with the same name are told apart by this. */
export function settingsPath(e: Pick<SettingsIndexEntry, "section" | "group" | "label">): string[] {
    const section = SETTINGS_SECTION_LABELS[e.section];
    const path = [section];
    if (e.group && e.group !== section && e.group !== e.label) path.push(e.group);
    path.push(e.label);
    return path;
}

const textOf = (e: SettingsIndexEntry): string[] => [e.label, ...e.keywords, e.description ?? "", settingsPath(e).join(" ")];

/**
 * The Settings search (§3.5), best first. Literal first, like the My Agents
 * filter: an entry whose label contains the query comes before one that only
 * matches in its curated keywords (the synonyms, §3.4) or description, and an
 * exact label comes first of all. Ties keep `entries` order. Only when no
 * entry contains the query does fuzzy matching step in, for a typo.
 */
export function searchSettings(entries: SettingsIndexEntry[], query: string): SettingsIndexEntry[] {
    const q = query.trim().toLowerCase();
    const literal = literalFirstSearch(entries, query, textOf, { keys: FUZZY_KEYS });
    const words = q.split(/\s+/).filter(Boolean);
    if (words.length < 2) return literal;
    // The path is searched too: "sounds volume" finds Volume under Sounds, though no one string holds both words.
    // The setting's own words (not its path) holding the whole phrase is a literal match; keep that order.
    if (literal.some((e) => textOf(e).slice(0, -1).some((t) => t.toLowerCase().includes(q)))) return literal;
    const together = entries.filter((e) => {
        const hay = textOf(e).join(" ").toLowerCase();
        return words.every((w) => hay.includes(w));
    });
    return together.length > 0 ? together : literal;
}
