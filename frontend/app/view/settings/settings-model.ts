// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";

export type SettingsSection =
    | "appearance"
    | "window"
    | "terminal"
    | "sounds"
    | "recording"
    | "advanced";

// Label text only, hoisted here so viewName can read it without importing
// from settings-view.tsx, which would reintroduce the circular import
// settings.tsx exists to avoid. settings-view.tsx's RAIL references this
// too, so the two can never drift out of sync.
export const SETTINGS_SECTION_LABELS: Record<SettingsSection, string> = {
    appearance: "Appearance",
    window: "Window & Panes",
    terminal: "Terminal",
    sounds: "Sounds",
    recording: "Recording",
    advanced: "Advanced",
};

export class SettingsViewModel implements ViewModel {
    viewType = "settings";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon = () => "cog";
    // wired in settings.tsx to avoid circular import
    declare viewComponent: ViewComponent<SettingsViewModel>;

    activeSection: () => SettingsSection;
    setSection: (s: SettingsSection) => void;
    viewName: () => string;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        const [section, setSection] = createSignal<SettingsSection>("appearance");
        this.activeSection = section;
        this.setSection = setSection;
        // No blockAtom/meta-persistence here (unlike Armory/Warden's
        // sectionAtom) — SettingsViewModel has never had a blockAtom, and
        // activeSection is already a plain createSignal read directly, so no
        // useBlockAtom wrapper is needed either (same as agent-model.ts's
        // viewName, which reads its own already-tracked signal inline).
        this.viewName = () => SETTINGS_SECTION_LABELS[this.activeSection()];
    }
}
