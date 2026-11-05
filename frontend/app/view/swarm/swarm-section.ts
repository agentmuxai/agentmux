// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's sections, as data (swarm.tsx declares them without loading the pane) and which of them
// are open. Kept apart from swarm-model.ts, which is at its size limit.
//
// The Swarm shows two collapsible sections above its list, Broadcast and Supervise; both can be open at
// once. The open ones are the pane's meta (`swarm:open`). `swarm:section` still names one to open, so a
// widget-bar entry, a command, a link or an agent can open the Swarm on it.

import { createMemo, type Accessor } from "solid-js";
import { hasSection, sectionKeyOf, type PaneSections } from "@/app/store/pane-sections";

export type SwarmSection = "supervisor" | "broadcast";

// Label text only, so the title can read it without importing the view. The id stays `supervisor`:
// saved panes, links and the settings page name it.
export const SWARM_SECTION_LABELS: Record<SwarmSection, string> = {
    supervisor: "Supervise",
    broadcast: "Broadcast",
};

/** The Swarm's sections (pane-sections.ts). The widget SDK lists them in this order; the panel draws Broadcast first. */
export const SWARM_SECTIONS: PaneSections<SwarmSection> = {
    default: "supervisor",
    items: [
        { id: "supervisor", label: SWARM_SECTION_LABELS.supervisor, icon: "user-shield" },
        { id: "broadcast", label: SWARM_SECTION_LABELS.broadcast, icon: "tower-broadcast" },
    ],
};

/** The pane's meta key for the open sections: an array of section ids. */
export const SWARM_OPEN_KEY = "swarm:open";

/** The rows' density: full rows, unless Broadcast is the only section open (compact ones). */
export type SwarmDensity = "full" | "compact";

export function swarmDensityOf(open: readonly SwarmSection[]): SwarmDensity {
    return open.includes("broadcast") && !open.includes("supervisor") ? "compact" : "full";
}

/** The open sections: those `swarm:open` lists, and the one `swarm:section` names (a link that
 *  opens the Swarm on a section opens it even in a pane that has saved its own). None by default. */
export function openSectionsOf(meta: Record<string, unknown> | null | undefined): SwarmSection[] {
    const saved = meta?.[SWARM_OPEN_KEY];
    const open = new Set<SwarmSection>(
        Array.isArray(saved) ? saved.filter((id): id is SwarmSection => hasSection(SWARM_SECTIONS, id)) : []
    );
    const named = meta?.[sectionKeyOf("swarm")];
    if (hasSection(SWARM_SECTIONS, named)) open.add(named);
    return SWARM_SECTIONS.items.map((s) => s.id).filter((id) => open.has(id));
}

/** The meta that opens or closes `id`, leaving the other as it is. `swarm:section` is cleared: once a
 *  person has chosen, `swarm:open` is the whole answer. */
export function toggledMeta(meta: Record<string, unknown> | null | undefined, id: SwarmSection): Record<string, unknown> {
    const open = openSectionsOf(meta);
    const next = open.includes(id) ? open.filter((s) => s !== id) : [...open, id];
    return { [SWARM_OPEN_KEY]: next, [sectionKeyOf("swarm")]: null };
}

/** The pane's title: the Swarm. Its sections are open or shut inside it, they don't rename it. */
export const SWARM_TITLE = "Swarm";

/** The sections the pane's meta has open. */
export function createOpenSwarmSections(meta: Accessor<Record<string, unknown> | null | undefined>): Accessor<SwarmSection[]> {
    return createMemo<SwarmSection[]>((prev) => {
        const next = openSectionsOf(meta());
        return prev && prev.length === next.length && prev.every((id, i) => id === next[i]) ? prev : next;
    });
}
