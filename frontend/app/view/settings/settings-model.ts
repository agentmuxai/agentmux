// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";

export type SettingsSection =
    | "appearance"
    | "window"
    | "terminal"
    | "sounds"
    | "notifications"
    | "recording"
    | "advanced";

/** One searchable settings row. `keywords` is where "synonyms match" lives —
 *  see docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md §3.4. Declared
 *  once per row, colocated in each section file (named-key objects, e.g.
 *  `APPEARANCE_SETTINGS.theme`), and referenced by the row's own
 *  `<SettingRow>` JSX for `label`/`description` — one string, not a
 *  duplicate copy — so the index can never drift from what's on screen. */
export interface SettingsIndexEntry {
    id: string;
    label: string;
    description?: string;
    section: SettingsSection;
    keywords: string[];
}

// Label text only, hoisted here so viewName can read it without importing
// from settings-view.tsx, which would reintroduce the circular import
// settings.tsx exists to avoid. settings-view.tsx's RAIL references this
// too, so the two can never drift out of sync.
export const SETTINGS_SECTION_LABELS: Record<SettingsSection, string> = {
    appearance: "Appearance",
    window: "Window & Panes",
    terminal: "Terminal",
    sounds: "Sounds",
    notifications: "Notifications & Tray",
    recording: "Recording",
    advanced: "Advanced",
};

/** The settings pane's state behind its native pane tab (`settingsPaneTab`,
 *  settings.tsx): the open section and the search query. */
export class SettingsViewModel {
    activeSection: () => SettingsSection;
    setSection: (s: SettingsSection) => void;
    viewName: () => string;

    // Search query — docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md §3.5.
    // Same plain-createSignal treatment as activeSection (no blockAtom; see
    // that field's own comment below for why) — it's transient UI state, not
    // something that needs to persist across a pane reload.
    query: () => string;
    setQuery: (q: string) => void;

    constructor() {
        const [section, setSection] = createSignal<SettingsSection>("appearance");
        this.activeSection = section;
        this.setSection = setSection;
        // No blockAtom/meta-persistence here (unlike Armory/Warden's
        // sectionAtom) — SettingsViewModel has never had a blockAtom, and
        // activeSection is already a plain createSignal read directly, so no
        // useBlockAtom wrapper is needed either (same as agent-model.ts's
        // viewName, which reads its own already-tracked signal inline).
        this.viewName = () => SETTINGS_SECTION_LABELS[this.activeSection()];
        const [query, setQuery] = createSignal("");
        this.query = query;
        this.setQuery = setQuery;
    }
}
