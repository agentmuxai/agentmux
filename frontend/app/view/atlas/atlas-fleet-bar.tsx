// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the map's selection can do (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §5.6): the Swarm's own Select
 * all, Stop, Supervisor mode and Broadcast composer, on the Swarm's own selection. Nothing here is a second
 * path: the same components act through the same model, so Stop asks with the same list, a broadcast reaches
 * the same agents, and the result names each one, exactly as in the Swarm.
 */

import { Show, type Accessor, type JSX } from "solid-js";
import type { SwarmAgents } from "../swarm/swarm-agent-list";
import { BroadcastDock } from "../swarm/swarm-broadcast";
import { FleetResultPanel, SuperviseActions } from "../swarm/swarm-fleet-toolbar";
import type { SwarmViewModel } from "../swarm/swarm-model";
import { SwarmSelectToggle } from "../swarm/swarm-select-toggle";

export function AtlasFleetBar(props: {
    model: SwarmViewModel;
    swarm: Pick<SwarmAgents, "tree" | "shownSelectableKeys">;
    /** Select is on: the bar shows with nothing selected yet. */
    selecting: boolean;
    /** The selection keys of the nodes drawn, which Select all selects. */
    nodeKeys: Accessor<string[]>;
}): JSX.Element {
    const model = props.model;
    const open = () => props.selecting || model.selectedBlockIdsAtom().size > 0;
    return (
        <Show when={open() || model.lastFleetResultAtom()}>
            <div class="atlas-fleet" data-testid="atlas-fleet">
                <Show when={open()}>
                    <div class="atlas-fleet__row">
                        <SwarmSelectToggle model={model} ids={props.nodeKeys} />
                        <Show when={model.selectedBlockIdsAtom().size === 0}>
                            <span class="atlas-fleet__hint">Drag a box around agents, or click them.</span>
                        </Show>
                        <SuperviseActions model={model} allBlockIds={props.swarm.shownSelectableKeys} />
                    </div>
                    <BroadcastDock model={model} agents={props.swarm} />
                </Show>
                {/* A clean broadcast is in the composer's recent list while that shows; anything else says what it did. */}
                <FleetResultPanel model={model} quietWhenClean={open()} />
            </div>
        </Show>
    );
}
