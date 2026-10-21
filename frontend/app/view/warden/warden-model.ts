// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { sectionOf } from "@/app/block/pane-sections";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { createMemo, createSignal, type Accessor } from "solid-js";
import { readZoom } from "@/app/store/zoom-factor";

export { WARDEN_SECTION_LABELS, WARDEN_SECTIONS, type WardenSection } from "./warden-sections";
import { WARDEN_SECTIONS, WARDEN_SECTION_LABELS, type WardenSection } from "./warden-sections";

/** Warden's state behind its native pane tab (warden.tsx). */
export class WardenViewModel {
    viewType = "warden";
    blockId: string;
    /** Writes the block's meta — the host context's. */
    setMeta: (patch: Record<string, unknown>) => void;
    // Per-pane zoom, same term:zoom metadata + clamp range as Connectors/Memory/editor/
    // term/agent/swarm — see section-pane.tsx's zoomAtom for the precedent
    // this mirrors exactly.
    zoomAtom: Accessor<number>;
    // Selected section, meta-backed the same way as zoomAtom above —
    // moved here from a local createSignal in warden-view.tsx so viewName
    // (below) can react to it. Mirrors section-pane.tsx's sectionAtom.
    sectionAtom: Accessor<WardenSection>;

    viewName: Accessor<string>;
    /** How many Kanban tasks need the operator: shown in the title on the Kanban section
     *  (SPEC_WARDEN_KANBAN_2026_10_20.md §6.1). Set by the section while it is shown. */
    kanbanNeeds: Accessor<number>;
    setKanbanNeeds: (n: number) => void;
    /** A task another view asked to show (`kanban:open` meta, kanban-held.tsx), until the section opens it. */
    kanbanOpenAtom: Accessor<string | null>;
    /** The Kanban in one lane per repository (`kanban:lanes` meta). */
    kanbanLanesAtom: Accessor<boolean>;

    // A native pane tab (Pane Tab contract Phase 2c): built by `create(ctx)`;
    // its memos live in the instance's own root (host rule 8), so they no
    // longer need the per-block atom cache to survive.
    constructor(ctx: PaneTabHostContext) {
        this.blockId = ctx.blockId;
        this.setMeta = (patch) => void ctx.setMeta(patch);
        const meta = ctx.meta;
        this.zoomAtom = createMemo<number>(() => readZoom(meta()));
        this.sectionAtom = createMemo<WardenSection>(() => sectionOf(WARDEN_SECTIONS, "warden", meta()));
        this.kanbanOpenAtom = createMemo<string | null>(() => {
            const id = meta()?.["kanban:open"];
            return typeof id === "string" && id ? id : null;
        });
        this.kanbanLanesAtom = createMemo<boolean>(() => meta()?.["kanban:lanes"] === true);
        const [kanbanNeeds, setKanbanNeeds] = createSignal(0);
        this.kanbanNeeds = kanbanNeeds;
        this.setKanbanNeeds = setKanbanNeeds;
        this.viewName = createMemo<string>(() => {
            const section = this.sectionAtom();
            const n = kanbanNeeds();
            return section === "kanban" && n > 0 ? `Kanban · ${n} need${n === 1 ? "s" : ""} you` : WARDEN_SECTION_LABELS[section];
        });
    }
}
