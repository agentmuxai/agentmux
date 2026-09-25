// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
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

/** Warden's state behind its native pane tab (warden.tsx). */
export class WardenViewModel {
    viewType = "warden";
    blockId: string;
    /** Writes the block's meta — the host context's. */
    setMeta: (patch: Record<string, unknown>) => void;
    // Per-pane zoom, same term:zoom metadata + clamp range as Armory/editor/
    // term/agent/swarm — see armory-model.ts's zoomAtom for the precedent
    // this mirrors exactly.
    zoomAtom: Accessor<number>;
    // Selected rail section, meta-backed the same way as zoomAtom above —
    // moved here from a local createSignal in warden-view.tsx so viewName
    // (below) can react to it. Mirrors armory-model.ts's sectionAtom.
    sectionAtom: Accessor<WardenSection>;

    viewName: Accessor<string>;

    // A native pane tab (Pane Tab contract Phase 2c): built by `create(ctx)`;
    // its memos live in the instance's own root (host rule 8), so they no
    // longer need the per-block atom cache to survive.
    constructor(ctx: PaneTabHostContext) {
        this.blockId = ctx.blockId;
        this.setMeta = (patch) => void ctx.setMeta(patch);
        const meta = ctx.meta;
        this.zoomAtom = createMemo<number>(() => {
            const z = meta()?.["term:zoom"];
            if (typeof z !== "number" || isNaN(z)) return 1.0;
            return Math.max(0.5, Math.min(2.0, z));
        });
        this.sectionAtom = createMemo<WardenSection>(() => {
            const s = meta()?.["warden:section"];
            return isWardenSection(s) ? s : "host";
        });
        this.viewName = createMemo<string>(() => WARDEN_SECTION_LABELS[this.sectionAtom()]);
    }
}
