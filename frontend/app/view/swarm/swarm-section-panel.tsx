// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's two sections above the agent list, Broadcast and Supervise: each a header that opens
// and closes its body, both open at once if the person wants. Which are open is the pane's meta
// (`swarm:open`), so it stays across restarts; a link that names one (`swarm:section`) opens it.

import { Show, type Accessor, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import type { SwarmAgents } from "./swarm-agent-list";
import { BroadcastDock } from "./swarm-broadcast";
import { FleetResultPanel, SuperviseActions } from "./swarm-fleet-toolbar";
import type { SwarmViewModel } from "./swarm-model";
import { SWARM_SECTION_LABELS, type SwarmSection } from "./swarm-section";
import { SupervisorSettingsBar } from "./swarm-supervisor-settings";

const ICONS: Record<SwarmSection, string> = { broadcast: "tower-broadcast", supervisor: "user-shield" };

/** One section: a header that is a button, and its body while open. */
function SectionBlock(props: { id: SwarmSection; open: boolean; onToggle: () => void; children: JSX.Element }): JSX.Element {
    const bodyId = () => `swarm-section-${props.id}`;
    return (
        <section class="swarm-section-block" data-section={props.id}>
            <Button
                variant="row"
                class="swarm-section-block__head"
                aria-expanded={props.open}
                aria-controls={bodyId()}
                onClick={props.onToggle}
            >
                <i class={`fa-solid fa-chevron-${props.open ? "down" : "right"} swarm-section-block__chevron`} />
                <i class={`fa-solid fa-${ICONS[props.id]} swarm-section-block__icon`} />
                <span class="swarm-section-block__label">{SWARM_SECTION_LABELS[props.id]}</span>
            </Button>
            <Show when={props.open}>
                <div class="swarm-section-block__body" id={bodyId()}>
                    {props.children}
                </div>
            </Show>
        </section>
    );
}

export function SwarmSections(props: {
    model: SwarmViewModel;
    agents: SwarmAgents;
    open: Accessor<SwarmSection[]>;
    onToggle: (id: SwarmSection) => void;
}): JSX.Element {
    const { model, agents } = props;
    const isOpen = (id: SwarmSection) => props.open().includes(id);
    return (
        <div class="swarm-compartment">
            <SectionBlock id="broadcast" open={isOpen("broadcast")} onToggle={() => props.onToggle("broadcast")}>
                {/* The composer; the selection it sends to is ticked in the list below. */}
                <BroadcastDock model={model} agents={agents} />
            </SectionBlock>
            <SectionBlock id="supervisor" open={isOpen("supervisor")} onToggle={() => props.onToggle("supervisor")}>
                <SupervisorSettingsBar blockId={model.blockId} />
                {/* Outside the tree's own Show: agents on other instances can be selected when this
                    instance has none, and these are what act on them. */}
                <SuperviseActions model={model} allBlockIds={agents.shownSelectableKeys} />
            </SectionBlock>
            {/* What the last Broadcast, Stop or mode change did, per agent. */}
            <FleetResultPanel model={model} quietWhenClean={isOpen("broadcast")} />
        </div>
    );
}
