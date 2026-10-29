// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Warden's sections, as data: its manifest (warden.tsx) declares them without loading the pane.
// Only Jobs is left. The Vault, History and Audit are panes of their own, the Host is on the Swarm,
// and LAN, Ambient and Internet are in the status bar (pane-sections.ts says where each went).

import type { PaneSections } from "@/app/block/pane-sections";

export type WardenSection = "jobs";

export const WARDEN_SECTION_LABELS: Record<WardenSection, string> = {
    jobs: "Jobs",
};

/** Warden's sections, as its manifest declares them (pane-sections.ts). */
export const WARDEN_SECTIONS: PaneSections<WardenSection> = {
    default: "jobs",
    items: [
        // Everything that runs by itself (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3).
        { id: "jobs", label: WARDEN_SECTION_LABELS.jobs, icon: "calendar-check" },
    ],
};
