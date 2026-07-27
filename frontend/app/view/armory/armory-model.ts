// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { useBlockAtom } from "@/app/store/global";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, type Accessor } from "solid-js";

export type ArmorySection = "accounts" | "memory" | "skills" | "mcp" | "bundles";

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

    viewIcon = () => "vault";
    viewName = () => "Armory";
    // wired in armory.tsx to avoid circular import
    declare viewComponent: ViewComponent<ArmoryViewModel>;

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
    }
}
