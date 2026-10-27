// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's section tabs, along the top as in Warden: icons only when narrow. The selected
// section is the pane's meta (`swarm:section`), so a widget-bar entry, a command or an agent can
// open the Swarm on one.

import type { Accessor, JSX } from "solid-js";
import { TabbedPane, type TabItem } from "@/app/element/ui";
import { SWARM_SECTIONS, type SwarmSection } from "./swarm-section";

const RAIL: TabItem<SwarmSection>[] = [...SWARM_SECTIONS.items];

export function SwarmSectionBar(props: {
    section: Accessor<SwarmSection>;
    onChange: (id: SwarmSection) => void;
    /** The selected section's content. */
    children: JSX.Element;
}): JSX.Element {
    return (
        <TabbedPane
            items={RAIL}
            value={props.section()}
            onChange={props.onChange}
            ariaLabel="Swarm section"
            class="swarm-sections"
            panelClass="swarm-section-panel"
        >
            {props.children}
        </TabbedPane>
    );
}
