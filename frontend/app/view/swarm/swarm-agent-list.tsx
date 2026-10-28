// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's one list of agents, shared by its sections. It is rendered once: a section switch
// changes the area above it and the density the rows read, never the rows' elements, so a press
// is never lost because the pressed element was replaced before release (#5208).

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { MOS, atoms } from "@/app/store/global";
import { isAwaitingUser } from "@/app/store/swarm-line";
import { readZoom } from "@/app/store/zoom-factor";
import { selectTrace } from "@/app/util/select-trace";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { Show, createEffect, createMemo, onCleanup, type Accessor, type JSX } from "solid-js";
import { phaseToDisplayStatus } from "./swarm-display-status";
import { remoteAgentKey } from "./swarm-fleet-targets";
import { lastActiveMs } from "./swarm-last-active";
import { collectClearableRows, type AgentTreeNode, type SwarmViewModel } from "./swarm-model";
import type { SwarmOrderItem } from "./swarm-order";
import { useSwarmOrder } from "./swarm-order-hold";
import { remoteChip, remoteRows, remoteSections, type RemoteRow } from "./swarm-remote";
import { stoppedAgents } from "./swarm-runtime";
import { createLiveAgentRuntimeStore, openStoppedAgent } from "./swarm-runtime-live";
import { SwarmGroupByControl, SwarmSeatList, useSwarmSeats } from "./swarm-seats";
import type { SwarmDensity } from "./swarm-section";
import { chipStatus } from "./swarm-status-chip";
import { StoppedAgents } from "./swarm-stopped-agents";
import { AgentRowHost, RemoteAgentRowHost, agentRowKey, focusedActiveBlockId } from "./swarm-view";

/** The agents the Swarm lists, derived once for the list and for the sections' actions above it. */
export interface SwarmAgents {
    /** This instance's agents (the tree). */
    tree: Accessor<AgentTreeNode[]>;
    /** The tree's rows, each with the key the list orders and finds it by. */
    keyedRows: Accessor<(readonly [string, AgentTreeNode])[]>;
    rowsByKey: Accessor<Map<string, AgentTreeNode>>;
    /** Other instances' agents. */
    remote: Accessor<RemoteRow[]>;
    remoteByKey: Accessor<Map<string, RemoteRow>>;
    /** Completed rows "Clear completed" would dismiss. */
    clearableCount: Accessor<number>;
    /** Created agents with no pane and no process (swarm-runtime.ts). */
    stopped: Accessor<AgentRuntime[]>;
    /** Everything a checkbox can select: this instance's agents, then the remote ones not gone
     *  stale. Select all and the selection both follow it. */
    selectableKeys: Accessor<string[]>;
}

export function useSwarmAgents(model: SwarmViewModel, ctx: PaneTabHostContext): SwarmAgents {
    // A Swarm pane nobody can see (another window tab, or a pane tab behind its
    // siblings) keeps its last list and rebuilds once when it is shown: every
    // agent event rebuilt every Swarm pane in the window, hidden ones included
    // (150 to 380 rebuilds a minute across four panes in one busy window).
    const tree = createMemo<AgentTreeNode[]>((prev) => {
        if (ctx.visibility() !== "active") return prev ?? [];
        const rows = model.buildTree();
        selectTrace.rebuilt();
        return rows;
    });
    // The list is keyed by block id, so a rebuild with the same agents updates the
    // rows in place; only an agent joining or leaving adds or removes a row.
    const keyedRows = createMemo(() => {
        const seen = new Map<string, number>();
        return tree().map((n) => {
            const base = agentRowKey(n);
            const count = (seen.get(base) ?? 0) + 1;
            seen.set(base, count);
            return [count === 1 ? base : `${base}#${count}`, n] as const;
        });
    });
    const rowsByKey = createMemo(() => new Map(keyedRows()));
    const clearableCount = createMemo(() => collectClearableRows(tree()).length);
    const remote = createMemo(() => remoteRows(remoteSections(model.otherInstancesAtom())));
    const remoteByKey = createMemo(
        () => new Map(remote().map((r) => [remoteAgentKey(r.section, r.agent), r] as const))
    );
    const localKeys = createMemo(() => tree().flatMap((n) => (n.blockId ? [n.blockId] : [])));
    // Every created agent's runtime state, for the ones with no pane (swarm-runtime.ts).
    const runtime = createLiveAgentRuntimeStore();
    void runtime.load();
    onCleanup(runtime.dispose);
    const stopped = createMemo(() => stoppedAgents(runtime.rows(), new Set(localKeys())));
    const selectableKeys = createMemo(() => [
        ...localKeys(),
        ...remote().flatMap((r) => (r.section.stale ? [] : [remoteAgentKey(r.section, r.agent)])),
    ]);
    // An agent that leaves the list leaves the selection, so the toolbar never
    // counts one no checkbox shows (docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md §2).
    createEffect(() => {
        if (!model.loadingAtom()) model.pruneSelection(localKeys());
    });
    return { tree, keyedRows, rowsByKey, remote, remoteByKey, clearableCount, stopped, selectableKeys };
}

export function SwarmAgentList(props: {
    model: SwarmViewModel;
    ctx: PaneTabHostContext;
    agents: SwarmAgents;
    /** Full rows in the Supervisor section, compact ones in Broadcast. */
    density: Accessor<SwarmDensity>;
}): JSX.Element {
    const model = props.model;
    const { keyedRows, rowsByKey, remote, remoteByKey, tree } = props.agents;
    const zoomFactor = createMemo(() => readZoom(props.ctx.meta()));
    // The order the rows are shown in: working agents first, the rest by last activity
    // (swarm-order.ts). A change waits, and waits longer while the list is in use, so a
    // row never moves under the cursor (swarm-order-hold.ts).
    let treeRef: HTMLDivElement | undefined;
    const swarmOrder = useSwarmOrder({
        list: () => treeRef,
        zoom: zoomFactor,
        items: () => [
            ...keyedRows().map(([key, n]): SwarmOrderItem => {
                const meta = n.blockId
                    ? MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", n.blockId))()?.meta
                    : undefined;
                const status = phaseToDisplayStatus(n.blockId, n.agentStatus);
                return {
                    key,
                    remote: false,
                    working: chipStatus(status, isAwaitingUser(meta, n.agentStatus)) !== "idle",
                    lastActive: lastActiveMs(meta),
                    name: n.agentName,
                };
            }),
            ...remote().map((r): SwarmOrderItem => ({
                key: remoteAgentKey(r.section, r.agent),
                remote: true,
                working: remoteChip(r.agent.status, r.section.stale)?.status === "working",
                lastActive: 0,
                name: r.agent.name,
            })),
        ],
    });
    const seats = useSwarmSeats(props.ctx, keyedRows, remoteByKey); // Group by lead (swarm-seats.tsx)

    // Derive the currently-focused block ID from the active tab's layout model.
    // Switching a multi-tab pane to another tab keeps the SAME focused node and
    // only changes its active member, so focusedNode() (a memo returning the
    // node object) doesn't signal and a read of `.data.blockId` would stay on
    // the previous tab — the selection border stuck on one agent. Subscribing
    // to localTreeStateAtom (which every tab switch republishes) and reading
    // the active member the way the pane's own node model does
    // (layoutNodeModels.ts activeBlockId) keeps it current.
    const focusedBlockId = createMemo<string | null>(() => {
        const tabId = atoms.activeTabId();
        if (!tabId) return null;
        const layoutModel = getLayoutModelForTabById(tabId);
        if (!layoutModel) return null;
        return focusedActiveBlockId(layoutModel);
    });

    return (
        // One list: this instance's agents and the other instances', working ones
        // first (swarm-order.ts; docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md §5).
        <>
            <Show when={tree().length > 0 || remote().length > 0}>
                <SwarmGroupByControl seats={seats} />
                <div
                    class="swarm-tree"
                    data-density={props.density()}
                    ref={(el) => {
                        treeRef = el;
                        swarmOrder.bindList(el);
                    }}
                    {...swarmOrder.listHandlers}
                >
                    <SwarmSeatList seats={seats} order={swarmOrder.order}>
                        {(key) =>
                            // Which kind of row it is comes from the maps the rows are read from. A key in
                            // neither (the agent just left) has no row to show.
                            rowsByKey().has(key) ? (
                                <AgentRowHost
                                    rowKey={key}
                                    rows={rowsByKey}
                                    focusedBlockId={focusedBlockId}
                                    model={model}
                                />
                            ) : remoteByKey().has(key) ? (
                                <RemoteAgentRowHost rowKey={key} rows={remoteByKey} model={model} />
                            ) : null
                        }
                    </SwarmSeatList>
                </div>
            </Show>
            <StoppedAgents
                agents={props.agents.stopped}
                hasLive={() => tree().length > 0 || remote().length > 0}
                onOpen={openStoppedAgent}
            />
        </>
    );
}
