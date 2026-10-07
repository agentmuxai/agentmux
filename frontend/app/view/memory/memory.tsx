// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory: what agents know and carry. Global Memory (instructions composed into
// every agent's startup file), each agent's Personal Memory, Skills, and the
// Bundles that package them; the other half of the Armory it replaced
// (docs/specs/SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md).
// Named Knowledge until docs/specs/SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { BundleManager } from "@/app/view/bundle/bundle-manager";
import { GlobalBundleManager } from "@/app/view/global-bundle/global-bundle-manager";
import { NativeMemoryManager } from "@/app/view/native-memory/native-memory-manager";
import {
    LEGACY_KNOWLEDGE_SECTION_KEY,
    LEGACY_KNOWLEDGE_VIEW,
    MEMORY_SECTION_KEY,
    MEMORY_VIEW,
    type MemorySection,
} from "@/app/view/section-pane/panes";
import { SectionPaneModel, SectionPaneView, type SectionPaneSpec } from "@/app/view/section-pane/section-pane";
import { SkillManager } from "@/app/view/skill/skill-manager";

export const MEMORY_SPEC: SectionPaneSpec<MemorySection> = {
    view: MEMORY_VIEW,
    sectionKey: MEMORY_SECTION_KEY,
    legacySectionKeys: [LEGACY_KNOWLEDGE_SECTION_KEY],
    defaultSection: "global",
    ariaLabel: "Memory section",
    sections: [
        {
            id: "global",
            label: "Global",
            icon: "globe",
            tooltip: "Instructions composed into every agent's startup file",
            component: () => <GlobalBundleManager />,
        },
        {
            id: "personal",
            label: "Personal",
            icon: "brain",
            tooltip: "What each agent writes in its own memory, with history",
            component: () => <NativeMemoryManager />,
        },
        { id: "skills", label: "Skills", icon: "wand-magic-sparkles", component: () => <SkillManager /> },
        {
            id: "bundles",
            label: "Bundles",
            icon: "layer-group",
            tooltip: "Bundles: instructions, MCP servers, memory and skills to bind to any agent; imports Agent Bundle Format (ABF) files",
            highlight: true,
            component: () => <BundleManager />,
        },
    ],
};

export const memoryPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: MEMORY_VIEW,
    // Its former id, still in saved blocks and layouts.
    aliases: [LEGACY_KNOWLEDGE_VIEW],
    label: "Memory",
    icon: "brain",
    defaultHue: 150,
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    create: (ctx) => {
        const model = new SectionPaneModel(ctx, MEMORY_SPEC);
        return {
            component: () => <SectionPaneView model={model} />,
            liveTitle: () => ({ text: `Memory · ${model.viewName()}` }),
        };
    },
};
