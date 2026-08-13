// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { useBlockAtom } from "@/app/store/global";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, type Accessor } from "solid-js";

export type WardenSection = "host" | "lan" | "internet" | "audit" | "supervisor";

// Label text only (not icon), hoisted here so viewName can read it without
// importing from warden-view.tsx, which would reintroduce the circular
// import warden.tsx exists to avoid. warden-view.tsx's RAIL references this
// too, so the two can never drift out of sync. Mirrors armory-model.ts's
// ARMORY_SECTION_LABELS exactly.
export const WARDEN_SECTION_LABELS: Record<WardenSection, string> = {
    host: "Host",
    lan: "LAN",
    internet: "Internet",
    audit: "Audit",
    supervisor: "Supervisor",
};

function isWardenSection(v: unknown): v is WardenSection {
    return typeof v === "string" && Object.prototype.hasOwnProperty.call(WARDEN_SECTION_LABELS, v);
}

export class WardenViewModel implements ViewModel {
    viewType = "warden";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: Accessor<Block>;
    // Per-pane zoom, same term:zoom metadata + clamp range as Armory/editor/
    // term/agent/swarm — see armory-model.ts's zoomAtom for the precedent
    // this mirrors exactly.
    zoomAtom: Accessor<number>;
    // Selected rail section, meta-backed the same way as zoomAtom above —
    // moved here from a local createSignal in warden-view.tsx so viewName
    // (below) can react to it. Mirrors armory-model.ts's sectionAtom.
    sectionAtom: Accessor<WardenSection>;

    viewIcon = () => "shield-halved";
    // wired in warden.tsx to avoid circular import
    declare viewComponent: ViewComponent<WardenViewModel>;
    declare viewName: Accessor<string>;

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
        this.sectionAtom = useBlockAtom(blockId, "warden-section", () =>
            createMemo<WardenSection>(() => {
                const s = this.blockAtom()?.meta?.["warden:section"];
                return isWardenSection(s) ? s : "host";
            }),
        );
        this.viewName = useBlockAtom(blockId, "warden-view-name", () =>
            createMemo<string>(() => WARDEN_SECTION_LABELS[this.sectionAtom()]),
        );
    }
}
