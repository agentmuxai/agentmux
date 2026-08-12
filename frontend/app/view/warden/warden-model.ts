// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { useBlockAtom } from "@/app/store/global";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, type Accessor } from "solid-js";

export type WardenSection = "host" | "lan" | "internet" | "audit" | "supervisor";

export class WardenViewModel implements ViewModel {
    viewType = "warden";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: Accessor<Block>;
    // Per-pane zoom, same term:zoom metadata + clamp range as Armory/editor/
    // term/agent/swarm — see armory-model.ts's zoomAtom for the precedent
    // this mirrors exactly.
    zoomAtom: Accessor<number>;

    viewIcon = () => "shield-halved";
    viewName = () => "Warden";
    // wired in warden.tsx to avoid circular import
    declare viewComponent: ViewComponent<WardenViewModel>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));
        this.zoomAtom = useBlockAtom(blockId, "warden-zoom", () =>
            createMemo<number>(() => {
                const z = this.blockAtom()?.meta?.["term:zoom"];
                if (typeof z !== "number" || isNaN(z)) return 1.0;
                return Math.max(0.5, Math.min(2.0, z));
            }),
        );
    }
}
