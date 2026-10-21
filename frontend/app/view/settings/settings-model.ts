// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";

import { sectionKeyOf, sectionOf, type PaneSections } from "@/app/block/pane-sections";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";

export type SettingsSection =
    | "appearance"
    | "window"
    | "browser"
    | "terminal"
    | "sounds"
    | "notifications"
    | "recording"
    | "devices"
    | "widgets"
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
    browser: "Browser",
    terminal: "Terminal",
    sounds: "Sounds",
    notifications: "Notifications & Tray",
    recording: "Recording",
    devices: "Paired devices",
    widgets: "Widgets",
    advanced: "Advanced",
};

/** Settings' sections, in rail order, as its manifest declares them (pane-sections.ts). */
export const SETTINGS_SECTIONS: PaneSections<SettingsSection> = {
    default: "appearance",
    items: [
        { id: "appearance", label: SETTINGS_SECTION_LABELS.appearance, icon: "palette" },
        { id: "window", label: SETTINGS_SECTION_LABELS.window, icon: "table-cells" },
        { id: "browser", label: SETTINGS_SECTION_LABELS.browser, icon: "globe" },
        { id: "terminal", label: SETTINGS_SECTION_LABELS.terminal, icon: "terminal" },
        { id: "sounds", label: SETTINGS_SECTION_LABELS.sounds, icon: "volume-high" },
        { id: "notifications", label: SETTINGS_SECTION_LABELS.notifications, icon: "bell" },
        { id: "recording", label: SETTINGS_SECTION_LABELS.recording, icon: "microphone" },
        { id: "devices", label: SETTINGS_SECTION_LABELS.devices, icon: "mobile-screen" },
        { id: "widgets", label: SETTINGS_SECTION_LABELS.widgets, icon: "puzzle-piece" },
        { id: "advanced", label: SETTINGS_SECTION_LABELS.advanced, icon: "sliders" },
    ],
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

    /** `block`: the pane's meta, which holds the open section under `settings:section` like every
     *  sectioned pane's, so Browser's Manage profiles… or a widget-bar entry can open it on one, an
     *  open Settings follows when asked for another, and the section survives a remount. Without it
     *  (tests), the section is local state. */
    constructor(block?: Pick<PaneTabHostContext, "meta" | "setMeta">) {
        if (block) {
            this.activeSection = () => sectionOf(SETTINGS_SECTIONS, "settings", block.meta());
            this.setSection = (s) => void block.setMeta({ [sectionKeyOf("settings")]: s });
        } else {
            const [section, setSection] = createSignal<SettingsSection>(SETTINGS_SECTIONS.default);
            this.activeSection = section;
            this.setSection = setSection;
        }
        this.viewName = () => SETTINGS_SECTION_LABELS[this.activeSection()];
        const [query, setQuery] = createSignal("");
        this.query = query;
        this.setQuery = setQuery;
    }
}
