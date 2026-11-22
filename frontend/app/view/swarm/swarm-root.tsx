// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { holdPaneContent, trackPaneContent } from "@/app/store/pane-content-holds";
import { createEffect, createMemo, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { SwarmViewModel } from "./swarm-model";
import { AccountAvailabilityStrip } from "@/app/view/account-availability/account-availability-strip";
import { selectTrace } from "@/app/util/select-trace";
import { readZoom } from "@/app/store/zoom-factor";
import { SwarmAgentList, useSwarmAgents, type SwarmAgents } from "./swarm-agent-list";
import type { SwarmFilterFocus } from "./swarm-filter-bar";
import { ClearCompletedButton } from "./swarm-view";
import { SwarmSections } from "./swarm-section-panel";
import { RegistrationFlagsProvider } from "./swarm-row-registration";
import { createOpenSwarmSections, swarmDensityOf, toggledMeta, type SwarmSection } from "./swarm-section";
import { EmptyState } from "@/app/element/ui";
import "./swarm-view.scss";

/** What a section shows with the shared list: an empty-state note or the "Clear completed" button. */
function SectionListNotes(props: { model: SwarmViewModel; agents: SwarmAgents }): JSX.Element {
    const { model, agents } = props;
    return (
        <Show
            when={agents.tree().length > 0}
            fallback={
                // Other instances' agents follow in the same list: a line, not a pane-high placeholder.
                <EmptyState
                    variant={agents.remote().length > 0 ? "inline" : "fill"}
                    icon="hexagon"
                    title="No active agent panes"
                >
                    Start an agent session to see it here.
                </EmptyState>
            }
        >
            <Show when={agents.clearableCount() > 0}>
                <ClearCompletedButton model={model} count={agents.clearableCount()} />
            </Show>
        </Show>
    );
}

export function SwarmView(props: {
    model: SwarmViewModel;
    ctx: PaneTabHostContext;
    /** Gets the filter box's focus handle, for the pane's Find (Ctrl/Cmd+F). */
    filterFocusRef?: (focus: SwarmFilterFocus) => void;
}): JSX.Element {
    const model = props.model;
    const ctx = props.ctx;
    // "Loading…" until the first list arrives: hold the pane until then
    // (pane-content-holds.ts).
    onCleanup(trackPaneContent(model.blockId));
    const releaseContent = holdPaneContent(model.blockId);
    onCleanup(releaseContent);
    createEffect(() => {
        if (!model.loadingAtom()) releaseContent();
    });

    const zoomFactor = createMemo(() => readZoom(ctx.meta()));
    const open = createOpenSwarmSections(ctx.meta);
    const density = createMemo(() => swarmDensityOf(open()));
    const toggle = (id: SwarmSection) => void ctx.setMeta(toggledMeta(ctx.meta(), id));
    // Trace presses on agent rows whose click never arrives (select-trace.ts).
    // Capture listeners on the root, which a rebuild of the rows never replaces.
    onMount(() => {
        const root = rootRef;
        if (!root) return;
        const onDown = (e: PointerEvent) => {
            if (e.button !== 0) return;
            const card = (e.target as Element | null)?.closest<HTMLElement>(".swarm-agent-card");
            const blockId = card?.dataset.blockId;
            if (card && blockId) selectTrace.pressed(blockId, card);
        };
        const onUp = () => selectTrace.released();
        const onCancel = () => selectTrace.cancelled();
        const onClick = (e: MouseEvent) => {
            const card = (e.target as Element | null)?.closest<HTMLElement>(".swarm-agent-card") ?? null;
            selectTrace.clicked(card?.dataset.blockId ?? null, card);
        };
        root.addEventListener("pointerdown", onDown, true);
        root.addEventListener("pointerup", onUp, true);
        root.addEventListener("pointercancel", onCancel, true);
        root.addEventListener("click", onClick, true);
        onCleanup(() => {
            root.removeEventListener("pointerdown", onDown, true);
            root.removeEventListener("pointerup", onUp, true);
            root.removeEventListener("pointercancel", onCancel, true);
            root.removeEventListener("click", onClick, true);
        });
    });
    // Ctrl+scroll and Ctrl/Cmd +/-/0 zoom this pane through the shared handlers
    // (app.tsx, keymodel.ts): the `paneZoom` capability is all it needs.

    let rootRef: HTMLDivElement | undefined;

    const agents = useSwarmAgents(model, ctx);

    return (
        <div ref={rootRef} class="swarm-view" style={{ zoom: zoomFactor() }} tabIndex={-1}>
            <Show
                when={!model.loadingAtom()}
                fallback={<div class="swarm-loading">Loading…</div>}
            >
                <RegistrationFlagsProvider visible={() => ctx.visibility() === "active"}>
                    <AccountAvailabilityStrip />
                    {/* The area above the list follows the open sections; the list itself is rendered once and
                        never remounted by opening or closing one, which is what keeps a press on a row from
                        being lost (#5208). */}
                    <div class="swarm-section-body">
                        <div class="swarm-agents-pane">
                            <SwarmSections model={model} agents={agents} open={open} onToggle={toggle} />
                            <SectionListNotes model={model} agents={agents} />
                            <SwarmAgentList model={model} ctx={ctx} agents={agents} density={density} filterFocusRef={props.filterFocusRef} />
                        </div>
                    </div>
                </RegistrationFlagsProvider>
            </Show>
        </div>
    );
}
