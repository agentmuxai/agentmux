// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { holdPaneContent, trackPaneContent } from "@/app/store/pane-content-holds";
import { createEffect, createMemo, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { SwarmViewModel } from "./swarm-model";
import { AccountAvailabilityStrip } from "@/app/view/account-availability/account-availability-strip";
import { selectTrace } from "@/app/util/select-trace";
import { readZoom } from "@/app/store/zoom-factor";
import { SwarmAgentList, useSwarmAgents, type SwarmAgents } from "./swarm-agent-list";
import { SwarmSectionBar } from "./swarm-section-bar";
import { createSwarmSection, swarmDensityOf } from "./swarm-section";
import { FleetToolbar, FleetResultPanel } from "./swarm-fleet-toolbar";
import "./swarm-view.scss";

// The rows and their buckets live in swarm-agent-row.tsx; this module keeps its old exports.
export * from "./swarm-agent-row";
export { focusedActiveBlockId } from "./swarm-agent-list";

/** What a section shows above the shared list: the fleet toolbar and its result, then either an
 *  empty-state note or the "Clear completed" button. */
function SectionActions(props: { model: SwarmViewModel; agents: SwarmAgents }): JSX.Element {
    const { model, agents } = props;
    return (
        <>
            {/* Outside the tree's own Show: agents on other instances can be selected
                when this instance has none, and the toolbar is what acts on them. It
                shows itself only when there is something to select or act on. */}
            <FleetToolbar model={model} allBlockIds={agents.selectableKeys} />
            <FleetResultPanel model={model} />
            <Show
                when={agents.tree().length > 0}
                fallback={
                    <div classList={{ "swarm-empty-root": true, "swarm-empty-root--compact": agents.remote().length > 0 }}>
                        <span class="swarm-empty-icon">⬡</span>
                        <div class="swarm-empty-title">No active agent panes</div>
                        <div class="swarm-empty-desc">
                            Start an agent session to see it here.
                        </div>
                    </div>
                }
            >
                <Show when={agents.clearableCount() > 0}>
                    <div class="swarm-toolbar">
                        <button
                            type="button"
                            class="swarm-clear-completed-btn"
                            title="Dismiss every completed/interrupted row currently shown"
                            onClick={() => model.retireAllCompleted()}
                        >
                            Clear completed ({agents.clearableCount()})
                        </button>
                    </div>
                </Show>
            </Show>
        </>
    );
}

export function SwarmView(props: { model: SwarmViewModel; ctx: PaneTabHostContext }): JSX.Element {
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
    const section = createSwarmSection(ctx.meta);
    const density = createMemo(() => swarmDensityOf(section()));
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
                <AccountAvailabilityStrip />
                <SwarmSectionBar section={section} onChange={(id) => void ctx.setMeta({ "swarm:section": id })}>
                    {/* The body is the same elements in every section: the area above the list
                        follows the section, the list itself is rendered once and never remounted
                        by a switch, which is what keeps a press on a row from being lost (#5208). */}
                    <div class="swarm-section-body">
                        <SectionActions model={model} agents={agents} />
                        <SwarmAgentList model={model} ctx={ctx} agents={agents} density={density} />
                    </div>
                </SwarmSectionBar>
            </Show>
        </div>
    );
}
