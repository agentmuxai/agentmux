// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabHostContext, PaneTabManifest } from "@/app/block/pane-tab-registry";
import { QuickTips } from "@/app/element/quicktips";
import { FilterInput } from "@/app/element/ui";
import { readZoom, writePaneZoom } from "@/app/store/zoom-factor";
import { createMemo, createSignal, onMount, type JSX } from "solid-js";

/**
 * Help — QuickTips with a filter box pinned on top (SPEC_HELP_PANE_FILTER_2026_10_08.md)
 * that is the pane's typing target, so opening Help and typing filters it. It zooms
 * through the shared pane zoom (`term:zoom`: Ctrl/Cmd +/-/0, Ctrl+scroll, the
 * all-panes gesture and reset), which it joins with its `paneZoom` capability; the
 * filter bar stays at 100%. The first NATIVE pane tab (Pane Tab contract Phase 2b,
 * SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §4): it reaches its block only
 * through the host context, never MOS/RpcApi/nodeModel directly.
 */
export const helpPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "help",
    label: "Help",
    icon: "circle-question",
    defaultHue: 90,
    capabilities: { paneZoom: {} },
    create: () => ({ component: HelpView }),
};

function HelpView(props: { ctx: PaneTabHostContext }): JSX.Element {
    const ctx = props.ctx;
    const [filter, setFilter] = createSignal("");
    const zoom = createMemo(() => readZoom(ctx.meta()));

    // Help kept its zoom in its own `help:zoom` until it joined the shared zoom:
    // move a saved level across once, so nobody's Help zoom is lost.
    // SPEC_SYSINFO_TEXT_ZOOM_DENSITY_AND_ONE_ZOOM_PATH_2026_10_18.md §3 (A.5).
    onMount(() => {
        const meta = ctx.meta();
        const legacy = meta?.["help:zoom"];
        if (typeof legacy !== "number") return;
        if (meta?.["term:zoom"] == null) writePaneZoom((m) => ctx.setMeta(m), legacy);
        void ctx.setMeta({ "help:zoom": null });
    });

    return (
        <div class="overflow-auto w-full h-full outline-none" tabIndex={0}>
            <div class="help-filter-bar sticky top-0 z-[1] flex px-[5px] pt-[10px] pb-[6px] bg-[var(--main-bg-color)]">
                <FilterInput value={filter()} onInput={setFilter} placeholder="Filter shortcuts and tips" ariaLabel="Filter help" />
            </div>
            {/* CSS zoom (not font-size) is intentional: scales the full layout box so
                @container breakpoints respond and the grid reflows responsively.
                Chromium handles CSS zoom correctly; font-size only scales text. */}
            <div style={{ zoom: zoom(), padding: "4px 5px 10px" }}>
                <QuickTips filter={filter()} />
            </div>
        </div>
    );
}

export { HelpView };
