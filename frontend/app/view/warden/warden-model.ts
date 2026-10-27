// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { sectionOf } from "@/app/block/pane-sections";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { vaultHostIn } from "@/app/view/vault/vault-in-warden";
import { VaultViewModel } from "@/app/view/vault/vault-model";
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
    /** How many kinds of AgentMux's own model calls are failing: shown in the title on the Ambient
     *  section. Set by the section while it is shown. */
    ambientFailing: Accessor<number>;
    setAmbientFailing: (n: number) => void;
    /** How many jobs are failing: shown in the title on the Jobs section. Set by the section while it is shown. */
    jobsFailing: Accessor<number>;
    setJobsFailing: (n: number) => void;
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
        const [ambientFailing, setAmbientFailing] = createSignal(0);
        this.ambientFailing = ambientFailing;
        this.setAmbientFailing = setAmbientFailing;
        const [jobsFailing, setJobsFailing] = createSignal(0);
        this.jobsFailing = jobsFailing;
        this.setJobsFailing = setJobsFailing;
        this.vault = new VaultViewModel(vaultHostIn(ctx, () => this.sectionAtom() === "vault"));
        this.viewName = createMemo<string>(() => {
            const section = this.sectionAtom();
            if (section === "vault") return `Vault · ${this.vault.viewName()}`;
            if (section === "ambient" && ambientFailing() > 0) return `Ambient · ${ambientFailing()} failing`;
            if (section === "jobs" && jobsFailing() > 0) return `Jobs · ${jobsFailing()} failing`;
            return WARDEN_SECTION_LABELS[section];
        });
    }
}
