// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane's place in the stores it lives in, from mount to dispose.
 *
 * Registered with the document store and the pane-state store in one call, during the component
 * body and before any hook's `onMount` can dispatch: the stores throw on a dispatch to an
 * unregistered slot (a silent drop otherwise), and a dispatcher must never see the pane in one
 * store and not the other (agent-pane-registration.ts). The returned model is what hooks and views
 * dispatch through; its disposed flag flips before either store unregisters, so a deferred
 * dispatch landing in the cleanup window is dropped.
 *
 * Also registers the pane with the Swarm view (same path as the PTY/OSC flow) and the layout
 * store, and owns the `layoutView` signal the list renders from. `zoom` there is a no-op: the one
 * CSS `zoom` on `.agent-view-zoomed` does the scaling.
 */

import { createSignal, onCleanup, type Accessor } from "solid-js";
import { registerPane as registerLayoutPane, snapshot as layoutSnapshot, unregisterPane as unregisterLayoutPane, type LayoutView } from "@/app/store/agent-pane-layout-store";
import { registerPane, unregisterPane, type AgentPaneModel } from "@/app/store/agent-pane-registration";
import { registerActivity, unregisterActivity } from "@/app/store/agentActivity";
import { getRecentDispatches } from "@/app/store/command-source";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";
import { getTrail } from "@/log/render-trail";
import { handleAgentIdChange } from "@/app/view/term/termagent";

export function usePaneRegistration(opts: {
    blockId: string;
    agentId: string;
    agentName: Accessor<string>;
}): { paneModel: AgentPaneModel; layoutView: Accessor<LayoutView | null> } {
    const { blockId } = opts;
    const paneModel = registerPane(blockId, { agentId: opts.agentId });
    registerActivity(blockId, () => paneModel.state.turnPhase);
    handleAgentIdChange(blockId, opts.agentName());

    onCleanup(() => {
        // A pane torn down mid-turn is unexpected: the "went blank mid-stream" failure is a silent
        // dispose by an outer owner (block.tsx on blockData -> null) that throws nothing, so no
        // error boundary dumps the trail. The stack says which owner did it.
        const phase = paneModel.state.turnPhase;
        if (workingFromPhase(phase)) {
            console.warn(`[agent-view] DISPOSE UNEXPECTED(mid-turn) blockId=${blockId.slice(0, 7)} turnPhase=${JSON.stringify(phase)} stack=${new Error().stack}`);
            try {
                console.warn(`[agent-view] DISPOSE mid-turn render_trail=${JSON.stringify(getTrail())}`);
                console.warn(`[agent-view] DISPOSE mid-turn recent_dispatches=${JSON.stringify(getRecentDispatches(40))}`);
            } catch {
                /* best-effort diagnostic */
            }
        }
        unregisterPane(blockId);
        unregisterActivity(blockId);
        handleAgentIdChange(blockId, undefined);
    });

    const [layoutView, setLayoutView] = createSignal<LayoutView | null>(null);
    registerLayoutPane(blockId, { layout: setLayoutView, zoom: () => {} });
    onCleanup(() => unregisterLayoutPane(blockId));
    // `__agentLayout()` in the console snapshots the slice, for CDP validation.
    if (import.meta.env.DEV) {
        (window as unknown as { __agentLayout?: () => unknown }).__agentLayout = () => layoutSnapshot(blockId);
    }

    return { paneModel, layoutView };
}
