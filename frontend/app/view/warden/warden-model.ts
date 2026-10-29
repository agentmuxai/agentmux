// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { createMemo, createSignal, type Accessor } from "solid-js";
import { readZoom } from "@/app/store/zoom-factor";

export { WARDEN_SECTION_LABELS, WARDEN_SECTIONS, type WardenSection } from "./warden-sections";
import { WARDEN_SECTION_LABELS } from "./warden-sections";

/** Warden's state behind its native pane tab (warden.tsx). */
export class WardenViewModel {
    viewType = "warden";
    blockId: string;
    // Per-pane zoom, the same term:zoom metadata + clamp range as the other panes.
    zoomAtom: Accessor<number>;
    /** Whether the pane is on screen (its tab active, its window shown). */
    shown: Accessor<boolean>;
    viewName: Accessor<string>;
    /** How many jobs are failing: shown in the title. Set by the Jobs manager while it is shown. */
    jobsFailing: Accessor<number>;
    setJobsFailing: (n: number) => void;

    constructor(ctx: PaneTabHostContext) {
        this.blockId = ctx.blockId;
        this.zoomAtom = createMemo<number>(() => readZoom(ctx.meta()));
        this.shown = createMemo(() => (ctx.visibility?.() ?? "active") === "active");
        const [jobsFailing, setJobsFailing] = createSignal(0);
        this.jobsFailing = jobsFailing;
        this.setJobsFailing = setJobsFailing;
        this.viewName = createMemo<string>(() =>
            jobsFailing() > 0 ? `${WARDEN_SECTION_LABELS.jobs} · ${jobsFailing()} failing` : WARDEN_SECTION_LABELS.jobs
        );
    }
}
