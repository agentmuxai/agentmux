// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Warden's sections, as data: its manifest (warden.tsx) declares them without loading the pane.

import type { PaneSections } from "@/app/block/pane-sections";

export type WardenSection = "kanban" | "host" | "lan" | "internet" | "audit" | "history";

// Label text only (not icon), hoisted here so viewName can read it without
// importing from warden-view.tsx, which would reintroduce the circular
// import warden.tsx exists to avoid. warden-view.tsx's RAIL references this
// too, so the two can never drift out of sync, like the section labels in
// connectors.tsx and memory.tsx.
export const WARDEN_SECTION_LABELS: Record<WardenSection, string> = {
    kanban: "Kanban",
    host: "Host",
    lan: "LAN",
    internet: "Internet",
    audit: "Audit",
    history: "History",
};

/** Warden's sections, in rail order, as its manifest declares them (pane-sections.ts). */
export const WARDEN_SECTIONS: PaneSections<WardenSection> = {
    default: "host",
    items: [
        { id: "kanban", label: WARDEN_SECTION_LABELS.kanban, icon: "table-columns" },
        { id: "host", label: WARDEN_SECTION_LABELS.host, icon: "server" },
        { id: "lan", label: WARDEN_SECTION_LABELS.lan, icon: "network-wired" },
        { id: "internet", label: WARDEN_SECTION_LABELS.internet, icon: "globe" },
        { id: "audit", label: WARDEN_SECTION_LABELS.audit, icon: "list-check" },
        { id: "history", label: WARDEN_SECTION_LABELS.history, icon: "clock-rotate-left" },
    ],
};
