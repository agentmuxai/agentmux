// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's sections, as data (swarm.tsx declares them without loading the pane) and the
// pane's selected one. Kept apart from swarm-model.ts, which is at its size limit.

import { createMemo, type Accessor } from "solid-js";
import { sectionOf, type PaneSections } from "@/app/block/pane-sections";

export type SwarmSection = "supervisor" | "broadcast" | "kanban";

// Label text only, so the title can read it without importing the view.
export const SWARM_SECTION_LABELS: Record<SwarmSection, string> = {
    supervisor: "Supervisor",
    broadcast: "Broadcast",
    kanban: "Kanban",
};

/** The Swarm's sections, in rail order (pane-sections.ts). */
export const SWARM_SECTIONS: PaneSections<SwarmSection> = {
    default: "supervisor",
    items: [
        { id: "supervisor", label: SWARM_SECTION_LABELS.supervisor, icon: "user-shield" },
        { id: "broadcast", label: SWARM_SECTION_LABELS.broadcast, icon: "tower-broadcast" },
        { id: "kanban", label: SWARM_SECTION_LABELS.kanban, icon: "table-columns" },
    ],
};

/** The rows' density: the Supervisor shows full rows, Broadcast compact ones. */
export type SwarmDensity = "full" | "compact";

export function swarmDensityOf(section: SwarmSection): SwarmDensity {
    return section === "broadcast" ? "compact" : "full";
}

/** The pane's title: the Swarm, then the section it shows; on the Kanban, how many tasks need
 *  the operator (SPEC_WARDEN_KANBAN_2026_10_20.md §6.1). */
export function swarmTitleOf(section: SwarmSection, kanbanNeeds = 0): string {
    const title = `Swarm · ${SWARM_SECTION_LABELS[section]}`;
    return section === "kanban" && kanbanNeeds > 0 ? `${title} · ${kanbanNeeds} need${kanbanNeeds === 1 ? "s" : ""} you` : title;
}

/** The section the pane's meta selects (`swarm:section`), or the default. */
export function createSwarmSection(meta: Accessor<Record<string, unknown> | null | undefined>): Accessor<SwarmSection> {
    return createMemo<SwarmSection>(() => sectionOf(SWARM_SECTIONS, "swarm", meta()));
}
