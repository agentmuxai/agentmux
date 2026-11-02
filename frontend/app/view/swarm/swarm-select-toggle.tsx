// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import type { SwarmViewModel } from "./swarm-model";
import { filterActive } from "./swarm-filter-state";

/**
 * "Select all" / "Select none" in one small toggle, under the filter and beside "Group by". It is
 * how a selection is started or cleared in one click, so it needs no selection to be there. `ids`
 * are the agents the filter shows: with a filter on it acts on those only, and the hidden ones
 * keep their ticks.
 */
export function SwarmSelectToggle(props: { model: SwarmViewModel; ids: () => string[] }): JSX.Element {
    const model = props.model;
    const filtering = () => filterActive(model);
    const allSelected = () => {
        const ids = props.ids();
        const selected = model.selectedBlockIdsAtom();
        return ids.length > 0 && ids.every((id) => selected.has(id));
    };
    const toggle = () => {
        if (!filtering()) return allSelected() ? model.clearSelection() : model.selectAll(props.ids());
        model.setManySelected(props.ids(), !allSelected());
    };
    return (
        <Show when={props.ids().length > 0}>
            <div class="swarm-select-toggle">
                <Button density="compact" class="swarm-select-toggle__btn" onClick={toggle}>
                    {allSelected() ? "Select none" : "Select all"}
                    <Show when={filtering()}> {props.ids().length} shown</Show>
                </Button>
            </div>
        </Show>
    );
}
