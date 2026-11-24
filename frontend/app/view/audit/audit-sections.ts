// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit pane's sections, as its manifest declares them (audit.tsx); the instance adds each one's component
// (audit-instance.tsx). No components here, so a model or a link can import it without loading the pane.

import type { PaneSections } from "@/app/store/pane-sections";

export const AUDIT_VIEW = "audit";
export type AuditSection = "events" | "conversations" | "typed" | "storage";

export const AUDIT_SECTIONS: PaneSections<AuditSection> = {
    default: "events",
    items: [
        { id: "events", label: "Events", icon: "list-check", tooltip: "Everything agents did: messages, actions, starts and ends, and what they did with your secrets" },
        { id: "conversations", label: "Conversations", icon: "users", tooltip: "Each agent's kept conversations, to open and read" },
        { id: "typed", label: "Typed", icon: "keyboard", tooltip: "What you typed to an agent" },
        { id: "storage", label: "Storage", icon: "hard-drive", tooltip: "Where history is kept, how much, and for how long" },
    ],
};
