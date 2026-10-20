// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { createMemo, createSignal, type Accessor } from "solid-js";
import { readZoom } from "@/app/store/zoom-factor";
import { isVaultPaneSection, SECTION_LABELS, type VaultPaneSection } from "./vault-kinds";

/** The Vault's state behind its native pane tab (vault.tsx). */
export class VaultViewModel {
    viewType = "vault";
    blockId: string;
    setMeta: (patch: Record<string, unknown>) => void;
    zoomAtom: Accessor<number>;
    /** The selected section, meta-backed so it survives a remount. */
    sectionAtom: Accessor<VaultPaneSection>;
    /** An agent's request to fill in, set when the request's prompt opens the Vault. */
    requestAtom: Accessor<string>;
    viewName: Accessor<string>;
    /** Whether the pane is on screen (its tab active, its window shown). */
    shown: Accessor<boolean>;
    /** The note on what may ask the person: open until they dismiss it. */
    promptsOpen: Accessor<boolean>;
    /** The export or import dialog, opened from the header. */
    transfer: Accessor<"export" | "import" | null>;
    openTransfer: (which: "export" | "import" | null) => void;

    constructor(ctx: PaneTabHostContext) {
        this.blockId = ctx.blockId;
        this.shown = createMemo(() => (ctx.visibility?.() ?? "active") === "active");
        this.setMeta = (patch) => void ctx.setMeta(patch);
        const meta = ctx.meta;
        this.zoomAtom = createMemo<number>(() => readZoom(meta()));
        this.sectionAtom = createMemo<VaultPaneSection>(() => {
            const s = meta()?.["vault:section"];
            return isVaultPaneSection(s) ? s : "personal";
        });
        this.requestAtom = createMemo<string>(() => {
            const r = meta()?.["vault:request"];
            return typeof r === "string" ? r : "";
        });
        this.viewName = createMemo<string>(() => SECTION_LABELS[this.sectionAtom()]);
        this.promptsOpen = createMemo<boolean>(() => meta()?.["vault:prompts-read"] !== true);
        const [transfer, setTransfer] = createSignal<"export" | "import" | null>(null);
        this.transfer = transfer;
        this.openTransfer = setTransfer;
    }
}
