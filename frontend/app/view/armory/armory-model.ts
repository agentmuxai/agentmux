// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { useBlockAtom } from "@/app/store/global";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, type Accessor } from "solid-js";

// Section ids are internal and stay stable across renames (they're
// persisted in block.meta["armory:section"] — changing an id would strand
// a user's previously-selected tab back to "accounts"). "memory" is the
// combined Memory tab (label "Memory" below), covering both the global
// brain (GlobalBrainManager, is_global Memory bundles) and the per-agent
// native memory history (NativeMemoryManager) as sections inside one pane
// — see docs/specs/SPEC_ARMORY_MEMORY_TAB_MERGE_2026_08_30.md. Distinct
// from a bundle's own "bundles" section (label "ABF"; its backing
// component, MemoryManager, is itself a naming leftover — see
// docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.3).
//
// "native_memory" is a legacy rail-section id: prior to the 08-30 merge it
// was its own rail tab ("Personal Memory") — see
// docs/specs/SPEC_ARMORY_MEMORY_GLOBAL_PERSONAL_RENAME_2026_08_22.md. It is
// no longer written, but a block saved before the merge may still carry it
// in block.meta["armory:section"]; sectionAtom below normalizes it to
// "memory" (and memorySubsectionAtom seeds "personal") so those users land
// on the equivalent spot in the merged tab instead of falling back to
// "accounts".
export type ArmorySection = "accounts" | "memory" | "skills" | "mcp" | "bundles";
type LegacyArmorySection = ArmorySection | "native_memory";

// Label text only (not icon/tooltip, which stay armory-view.tsx's own
// presentation detail) — hoisted here so viewName can read it without
// importing from armory-view.tsx, which would reintroduce the circular
// import armory.tsx exists to avoid. armory-view.tsx's RAIL references this
// too, so the two can never drift out of sync.
export const ARMORY_SECTION_LABELS: Record<ArmorySection, string> = {
    accounts: "Accounts",
    memory: "Memory",
    skills: "Skills",
    mcp: "MCP Servers",
    bundles: "ABF",
};

function isArmorySection(v: unknown): v is ArmorySection {
    return typeof v === "string" && Object.prototype.hasOwnProperty.call(ARMORY_SECTION_LABELS, v);
}

// Sub-tab inside the merged Memory pane — see armory-view.tsx's memory
// sub-nav and SPEC_ARMORY_MEMORY_TAB_MERGE_2026_08_30.md.
export type MemorySubsection = "global" | "personal";

function isMemorySubsection(v: unknown): v is MemorySubsection {
    return v === "global" || v === "personal";
}

export class ArmoryViewModel implements ViewModel {
    viewType = "armory";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: Accessor<Block>;
    // Per-pane zoom, same term:zoom metadata + clamp range as editor/term/
    // agent/swarm (editor-model.ts's zoomAtom is the direct precedent —
    // Armory reuses the key rather than introducing "armory:zoom", since
    // the key is already generic across four non-terminal view types).
    zoomAtom: Accessor<number>;
    // Selected rail section, meta-backed the same way as zoomAtom above —
    // moved here from a local createSignal in armory-view.tsx so viewName
    // (below) can react to it, and as a side effect the selected tab now
    // survives a block remount instead of always resetting to "accounts".
    sectionAtom: Accessor<ArmorySection>;
    // Sub-tab (Global vs Personal) inside the merged Memory pane — same
    // meta-backed pattern as sectionAtom. See
    // SPEC_ARMORY_MEMORY_TAB_MERGE_2026_08_30.md.
    memorySubsectionAtom: Accessor<MemorySubsection>;

    viewIcon = () => "vault";
    // wired in armory.tsx to avoid circular import
    declare viewComponent: ViewComponent<ArmoryViewModel>;
    declare viewName: Accessor<string>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));
        this.zoomAtom = useBlockAtom(blockId, "armory-zoom", () =>
            createMemo<number>(() => {
                const z = this.blockAtom()?.meta?.["term:zoom"];
                if (typeof z !== "number" || isNaN(z)) return 1.0;
                return Math.max(0.5, Math.min(2.0, z));
            }),
        );
        this.sectionAtom = useBlockAtom(blockId, "armory-section", () =>
            createMemo<ArmorySection>(() => {
                const s = this.blockAtom()?.meta?.["armory:section"] as LegacyArmorySection | undefined;
                if (s === "native_memory") return "memory";
                return isArmorySection(s) ? s : "accounts";
            }),
        );
        this.memorySubsectionAtom = useBlockAtom(blockId, "armory-memory-subsection", () =>
            createMemo<MemorySubsection>(() => {
                const sub = this.blockAtom()?.meta?.["armory:memory:subsection"];
                if (isMemorySubsection(sub)) return sub;
                // Legacy: a pre-merge armory:section of "native_memory" meant
                // the user had Personal Memory open — seed Personal instead
                // of defaulting to Global. See sectionAtom above.
                const legacySection = this.blockAtom()?.meta?.["armory:section"];
                return legacySection === "native_memory" ? "personal" : "global";
            }),
        );
        this.viewName = useBlockAtom(blockId, "armory-view-name", () =>
            createMemo<string>(() => ARMORY_SECTION_LABELS[this.sectionAtom()]),
        );
    }
}
