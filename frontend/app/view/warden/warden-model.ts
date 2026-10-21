// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { sectionOf, type PaneSections } from "@/app/block/pane-sections";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { vaultHostIn } from "@/app/view/vault/vault-in-warden";
import { VAULT_SECTIONS } from "@/app/view/vault/vault-kinds";
import { VaultViewModel } from "@/app/view/vault/vault-model";
import { createMemo, createSignal, type Accessor } from "solid-js";
import { readZoom } from "@/app/store/zoom-factor";

export type WardenSection = "kanban" | "vault" | "host" | "lan" | "internet" | "audit" | "history";

// Label text only (not icon), hoisted here so viewName can read it without
// importing from warden-view.tsx, which would reintroduce the circular
// import warden.tsx exists to avoid. warden-view.tsx's RAIL references this
// too, so the two can never drift out of sync, like the section labels in
// connectors.tsx and memory.tsx.
export const WARDEN_SECTION_LABELS: Record<WardenSection, string> = {
    kanban: "Kanban",
    vault: "Vault",
    host: "Host",
    lan: "LAN",
    internet: "Internet",
    audit: "Audit",
    history: "History",
};

/** Warden's sections, in rail order, as its manifest declares them (pane-sections.ts). */
export const WARDEN_SECTIONS: PaneSections<WardenSection> = {
    default: "host",
    items: [
        { id: "kanban", label: WARDEN_SECTION_LABELS.kanban, icon: "table-columns" },
        // The person's secrets: once a pane of its own (vault-in-warden.ts), with its own sections.
        { id: "vault", label: WARDEN_SECTION_LABELS.vault, icon: "lock", sections: VAULT_SECTIONS },
        { id: "host", label: WARDEN_SECTION_LABELS.host, icon: "server" },
        { id: "lan", label: WARDEN_SECTION_LABELS.lan, icon: "network-wired" },
        { id: "internet", label: WARDEN_SECTION_LABELS.internet, icon: "globe" },
        { id: "audit", label: WARDEN_SECTION_LABELS.audit, icon: "list-check" },
        { id: "history", label: WARDEN_SECTION_LABELS.history, icon: "clock-rotate-left" },
    ],
};

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
    /** The Vault section's own model, on this block (vault-in-warden.ts). */
    vault: VaultViewModel;

    // A native pane tab (Pane Tab contract Phase 2c): built by `create(ctx)`;
    // its memos live in the instance's own root (host rule 8), so they no
    // longer need the per-block atom cache to survive.
    constructor(ctx: PaneTabHostContext) {
        this.blockId = ctx.blockId;
        this.setMeta = (patch) => void ctx.setMeta(patch);
        const meta = ctx.meta;
        this.zoomAtom = createMemo<number>(() => readZoom(meta()));
        this.sectionAtom = createMemo<WardenSection>(() => {
            const m = meta();
            // A block saved as the Vault pane (the `vault` alias) opens on the Vault section.
            if (m?.view === "vault" && m["warden:section"] === undefined) return "vault";
            return sectionOf(WARDEN_SECTIONS, "warden", m);
        });
        this.kanbanOpenAtom = createMemo<string | null>(() => {
            const id = meta()?.["kanban:open"];
            return typeof id === "string" && id ? id : null;
        });
        this.kanbanLanesAtom = createMemo<boolean>(() => meta()?.["kanban:lanes"] === true);
        const [kanbanNeeds, setKanbanNeeds] = createSignal(0);
        this.kanbanNeeds = kanbanNeeds;
        this.setKanbanNeeds = setKanbanNeeds;
        this.vault = new VaultViewModel(vaultHostIn(ctx, () => this.sectionAtom() === "vault"));
        this.viewName = createMemo<string>(() => {
            const section = this.sectionAtom();
            const n = kanbanNeeds();
            if (section === "vault") return `Vault · ${this.vault.viewName()}`;
            return section === "kanban" && n > 0 ? `Kanban · ${n} need${n === 1 ? "s" : ""} you` : WARDEN_SECTION_LABELS[section];
        });
    }
}
