// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";

export type SettingsSection =
    | "appearance"
    | "terminal"
    | "agent"
    | "sounds"
    | "network"
    | "files"
    | "advanced";

export class SettingsViewModel implements ViewModel {
    viewType = "settings";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon = () => "cog";
    viewName = () => "Settings";
    // wired in settings.tsx to avoid circular import
    declare viewComponent: ViewComponent<SettingsViewModel>;

    activeSection: () => SettingsSection;
    setSection: (s: SettingsSection) => void;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        const [section, setSection] = createSignal<SettingsSection>("appearance");
        this.activeSection = section;
        this.setSection = setSection;
    }
}
