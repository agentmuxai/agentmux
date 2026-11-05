// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import type { SwarmViewModel } from "./swarm-model";
import { actionableKeys, filterActive } from "./swarm-filter-state";
import { remoteFleetTargets } from "./swarm-fleet-targets";
import { remoteSections } from "./swarm-remote";

/**
 * "Select all" / "Select none" in one small toggle, under the filter and beside "Group by", with how
 * many agents are selected right after it. It is how a selection is started or cleared in one click, so
 * it needs no selection to be there. `ids` are the agents the filter shows: with a filter on it acts
 * on those only, and the hidden ones keep their ticks.
 */
export function SwarmSelectToggle(props: { model: SwarmViewModel; ids: () => string[] }): JSX.Element {
    const model = props.model;
    const filtering = () => filterActive(model);
    const count = () => model.selectedBlockIdsAtom().size;
    const allSelected = () => {
        const ids = props.ids();
        const selected = model.selectedBlockIdsAtom();
        return ids.length > 0 && ids.every((id) => selected.has(id));
    };
    const toggle = () => {
        if (!filtering()) return allSelected() ? model.clearSelection() : model.selectAll(props.ids());
        model.setManySelected(props.ids(), !allSelected());
    };
    // What the selection is, in a line beside the button: how many, and which of them an action won't reach.
    const remote = createMemo(() => remoteFleetTargets(remoteSections(model.otherInstancesAtom())));
    const hidden = () => count() - actionableKeys(model, model.selectedBlockIdsAtom()).length;
    const otherMachines = () => {
        let n = 0;
        for (const key of actionableKeys(model, model.selectedBlockIdsAtom())) if (remote().get(key)?.otherMachine) n++;
        return n;
    };
    return (
        <Show when={props.ids().length > 0 || count() > 0}>
            <div class="swarm-select-toggle">
                <Show when={props.ids().length > 0}>
                    <Button density="compact" class="swarm-select-toggle__btn" onClick={toggle}>
                        {allSelected() ? "Select none" : "Select all"}
                        <Show when={filtering()}> {props.ids().length} shown</Show>
                    </Button>
                </Show>
                <Show when={count() > 0}>
                    <span class="swarm-select-toggle__stats" role="status">
                        {count()} selected
                        <Show when={hidden() > 0}> · {hidden()} hidden by the filter, not included</Show>
                        <Show when={otherMachines() > 0}> · {otherMachines()} on other machines</Show>
                    </span>
                </Show>
            </div>
        </Show>
    );
}
