// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, on } from "solid-js";

import { sectionKeyOf, sectionOf, type PaneSections } from "@/app/block/pane-sections";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { readZoom } from "@/app/store/zoom-factor";

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
    | "agents"
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
    /** The heading the row sits under inside its section ("Tool-call tones"), when it sits under one:
     *  the middle of the result's path, "Sounds › Tool-call tones › Volume". */
    group?: string;
    keywords: string[];
    /** An entry whose settings live in another pane: choosing it runs this instead of showing a row. */
    opens?: () => void;
    /** A developer option: the row, and so the search result, exist only while dev mode is on. */
    devOnly?: boolean;
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
    agents: "Agents",
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
        { id: "agents", label: SETTINGS_SECTION_LABELS.agents, icon: "robot" },
        { id: "advanced", label: SETTINGS_SECTION_LABELS.advanced, icon: "sliders" },
    ],
};

/** The settings pane's state behind its native pane tab (`settingsPaneTab`,
 *  settings.tsx): the open section and the search query. */
export class SettingsViewModel {
    activeSection: () => SettingsSection;
    setSection: (s: SettingsSection) => void;
    viewName: () => string;
    /** The pane's zoom (`term:zoom`), applied to the whole view; 1 without a block (tests). */
    zoom: () => number;

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
            // Shown at once, then saved: a search result in another section looks for its row
            // right after switching, before the meta write comes back. A change from outside
            // (another entry point asking for a section) still lands through the meta.
            const fromMeta = () => sectionOf(SETTINGS_SECTIONS, "settings", block.meta());
            const [section, setLocal] = createSignal<SettingsSection>(fromMeta());
            createEffect(on(fromMeta, setLocal, { defer: true }));
            this.activeSection = section;
            this.setSection = (s) => {
                setLocal(s);
                void block.setMeta({ [sectionKeyOf("settings")]: s });
            };
        } else {
            const [section, setSection] = createSignal<SettingsSection>(SETTINGS_SECTIONS.default);
            this.activeSection = section;
            this.setSection = setSection;
        }
        this.viewName = () => SETTINGS_SECTION_LABELS[this.activeSection()];
        this.zoom = () => readZoom(block?.meta());
        const [query, setQuery] = createSignal("");
        this.query = query;
        this.setQuery = setQuery;
    }
}
