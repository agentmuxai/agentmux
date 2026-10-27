// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { holdPaneContent, trackPaneContent } from "@/app/store/pane-content-holds";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, type Accessor, type JSX } from "solid-js";
import type { SwarmViewModel } from "./swarm-model";
import { AccountAvailabilityStrip } from "@/app/view/account-availability/account-availability-strip";
import { selectTrace } from "@/app/util/select-trace";
import { readZoom } from "@/app/store/zoom-factor";
import { SwarmAgentList, useSwarmAgents, type SwarmAgents } from "./swarm-agent-list";
import { ClearCompletedButton } from "./swarm-view";
import { SwarmSectionBar } from "./swarm-section-bar";
import { KanbanSection } from "@/app/view/warden-kanban/kanban-section";
import { createSwarmSection, swarmDensityOf, type SwarmSection } from "./swarm-section";
import { BroadcastDock } from "./swarm-broadcast";
import { SupervisorSettingsBar } from "./swarm-supervisor-settings";
import { FleetToolbar, FleetResultPanel } from "./swarm-fleet-toolbar";
import "./swarm-view.scss";

/** What a section shows above the shared list: the fleet toolbar and its result, then either an
 *  empty-state note or the "Clear completed" button. */
function SectionActions(props: { model: SwarmViewModel; agents: SwarmAgents; section: SwarmSection }): JSX.Element {
    const { model, agents } = props;
    return (
        <>
            <Show when={props.section === "supervisor"}>
                <SupervisorSettingsBar />
            </Show>
            {/* Outside the tree's own Show: agents on other instances can be selected
                when this instance has none, and the toolbar is what acts on them. It
                shows itself only when there is something to select or act on. */}
            <FleetToolbar model={model} allBlockIds={agents.selectableKeys} section={props.section} />
            <FleetResultPanel model={model} quietWhenClean={props.section === "broadcast"} />
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
                    <ClearCompletedButton model={model} count={agents.clearableCount()} />
                </Show>
            </Show>
        </>
    );
}

export function SwarmView(props: {
    model: SwarmViewModel;
    ctx: PaneTabHostContext;
    /** The Kanban section's Needs-you count, for the pane's title (swarm-instance.tsx). */
    kanbanNeeds?: Accessor<number>;
    onKanbanNeeds?: (n: number) => void;
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
    const section = createSwarmSection(ctx.meta);
    const density = createMemo(() => swarmDensityOf(section()));
    // The Kanban mounts the first time its section is shown and stays mounted (its polling follows
    // `visible`), as Warden's sections do. The agent list is never unmounted: it is hidden while the
    // Kanban shows, so no row is remounted by a switch (#5208).
    const [kanbanSeen, setKanbanSeen] = createSignal(section() === "kanban");
    createEffect(() => {
        if (section() === "kanban") setKanbanSeen(true);
    });
    const kanbanOpen = createMemo<string | null>(() => {
        const id = ctx.meta()?.["kanban:open"];
        return typeof id === "string" && id ? id : null;
    });
    // A card's agent: switch to the Supervisor and select that agent's row, scrolled into view. The
    // meta write is asynchronous and the list is `display: none` until the section has changed, so the
    // scroll waits for the section rather than for a frame.
    const [scrollTo, setScrollTo] = createSignal<string | null>(null);
    createEffect(() => {
        const blockId = scrollTo();
        if (!blockId || section() === "kanban") return;
        setScrollTo(null);
        requestAnimationFrame(() =>
            [...(rootRef?.querySelectorAll<HTMLElement>(".swarm-agent-card") ?? [])]
                .find((card) => card.dataset.blockId === blockId)
                ?.scrollIntoView?.({ block: "nearest" })
        );
    });
    const openAgent = (blockId: string) => {
        model.selectAll([blockId]);
        setScrollTo(blockId);
        void ctx.setMeta({ "swarm:section": "supervisor" });
    };
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
                        <div class="swarm-agents-pane" classList={{ "is-hidden": section() === "kanban" }}>
                            <SectionActions model={model} agents={agents} section={section()} />
                            <SwarmAgentList model={model} ctx={ctx} agents={agents} density={density} />
                            {/* After the list, so a switch never remounts it. */}
                            <Show when={section() === "broadcast"}>
                                <BroadcastDock model={model} agents={agents} />
                            </Show>
                        </div>
                        <Show when={kanbanSeen()}>
                            <div class="swarm-kanban-slot" classList={{ "is-hidden": section() !== "kanban" }}>
                                <KanbanSection
                                    visible={section() === "kanban"}
                                    openRequest={kanbanOpen()}
                                    onOpenHandled={() => void ctx.setMeta({ "kanban:open": null })}
                                    onNeedsCount={(n) => props.onKanbanNeeds?.(n)}
                                    lanes={ctx.meta()?.["kanban:lanes"] === true}
                                    onLanes={(on) => void ctx.setMeta({ "kanban:lanes": on })}
                                    onOpenAgent={openAgent}
                                />
                            </div>
                        </Show>
                    </div>
                </SwarmSectionBar>
            </Show>
        </div>
    );
}
