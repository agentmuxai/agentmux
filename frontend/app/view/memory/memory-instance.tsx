// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory: what agents know and carry. Global Memory (instructions composed into
// every agent's startup file), each agent's Personal Memory, Skills, and the
// Bundles that package them.

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { BundleManager } from "@/app/view/bundle/bundle-manager";
import { GlobalBundleManager } from "@/app/view/global-bundle/global-bundle-manager";
import { NativeMemoryManager } from "@/app/view/native-memory/native-memory-manager";
import { MEMORY_SECTIONS, MEMORY_VIEW, type MemorySection } from "@/app/view/section-pane/panes";
import { SectionPaneModel, SectionPaneView, sectionPaneSpec } from "@/app/view/section-pane/section-pane";
import { SkillManager } from "@/app/view/skill/skill-manager";

const MEMORY_SPEC = sectionPaneSpec<MemorySection>(MEMORY_VIEW, "Memory section", MEMORY_SECTIONS, {
    global: () => <GlobalBundleManager />,
    personal: (pane) => <NativeMemoryManager pane={pane} />,
    skills: () => <SkillManager />,
    bundles: () => <BundleManager />,
});

/** A Memory pane's instance, loaded with the first one (memory.tsx). */
export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new SectionPaneModel(ctx, MEMORY_SPEC);
    return {
        component: () => <SectionPaneView model={model} />,
        liveTitle: () => ({ text: `Memory · ${model.viewName()}` }),
    };
}
