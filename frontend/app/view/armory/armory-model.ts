// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { useBlockAtom } from "@/app/store/global";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, type Accessor } from "solid-js";

export type ArmorySection = "accounts" | "memory" | "skills" | "mcp" | "bundles";

// Label text only (not icon/tooltip, which stay armory-view.tsx's own
// presentation detail) — hoisted here so viewName can read it without
// importing from armory-view.tsx, which would reintroduce the circular
// import armory.tsx exists to avoid. armory-view.tsx's RAIL references this
// too, so the two can never drift out of sync.
export const ARMORY_SECTION_LABELS: Record<ArmorySection, string> = {
    accounts: "Accounts",
    memory: "Memories",
    skills: "Skills",
    mcp: "MCP Servers",
    bundles: "ABF",
};

function isArmorySection(v: unknown): v is ArmorySection {
    return typeof v === "string" && Object.prototype.hasOwnProperty.call(ARMORY_SECTION_LABELS, v);
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
                const s = this.blockAtom()?.meta?.["armory:section"];
                return isArmorySection(s) ? s : "accounts";
            }),
        );
        this.viewName = useBlockAtom(blockId, "armory-view-name", () =>
            createMemo<string>(() => ARMORY_SECTION_LABELS[this.sectionAtom()]),
        );
    }
}
