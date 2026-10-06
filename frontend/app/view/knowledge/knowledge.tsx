// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Knowledge: what agents know and carry. Global instructions, each agent's own
// (Personal) memory, Skills, and the Bundles that package them; the other half
// of the Armory it replaced
// (docs/specs/SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md).

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { BundleManager } from "@/app/view/bundle/bundle-manager";
import { GlobalBundleManager } from "@/app/view/global-bundle/global-bundle-manager";
import { NativeMemoryManager } from "@/app/view/native-memory/native-memory-manager";
import { KNOWLEDGE_SECTION_KEY, KNOWLEDGE_VIEW, type KnowledgeSection } from "@/app/view/section-pane/panes";
import { SectionPaneModel, SectionPaneView, type SectionPaneSpec } from "@/app/view/section-pane/section-pane";
import { SkillManager } from "@/app/view/skill/skill-manager";

export const KNOWLEDGE_SPEC: SectionPaneSpec<KnowledgeSection> = {
    view: KNOWLEDGE_VIEW,
    sectionKey: KNOWLEDGE_SECTION_KEY,
    defaultSection: "global",
    ariaLabel: "Knowledge section",
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

export const knowledgePaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: KNOWLEDGE_VIEW,
    label: "Knowledge",
    icon: "book",
    defaultHue: 150,
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    create: (ctx) => {
        const model = new SectionPaneModel(ctx, KNOWLEDGE_SPEC);
        return {
            component: () => <SectionPaneView model={model} />,
            liveTitle: () => ({ text: `Knowledge · ${model.viewName()}` }),
        };
    },
};
