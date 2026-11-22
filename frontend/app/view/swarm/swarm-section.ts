// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's title and the section names its manifest still declares (swarm.tsx declares them without
// loading the pane). The Swarm has no sections on screen any more: the Filter, the Broadcast input, the
// selection's row and the agents sit in one column. The names stay so that a saved pane, a link or a
// widget-bar entry that names one still opens the Swarm; the pane does not read it.

import type { PaneSections } from "@/app/store/pane-sections";

export type SwarmSection = "supervisor" | "broadcast";

// Label text only, so the title can read it without importing the view. The id stays `supervisor`:
// saved panes, links and the settings page name it.
export const SWARM_SECTION_LABELS: Record<SwarmSection, string> = {
    supervisor: "Supervise",
    broadcast: "Broadcast",
};

/** The names the manifest declares (pane-sections.ts). The widget SDK lists them in this order. */
export const SWARM_SECTIONS: PaneSections<SwarmSection> = {
    default: "supervisor",
    items: [
        { id: "supervisor", label: SWARM_SECTION_LABELS.supervisor, icon: "user-shield" },
        { id: "broadcast", label: SWARM_SECTION_LABELS.broadcast, icon: "tower-broadcast" },
    ],
};

/** The pane's title: the Swarm. */
export const SWARM_TITLE = "Swarm";
