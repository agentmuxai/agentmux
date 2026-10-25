// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Connectors and Memory panes: their view ids, section ids and meta keys, and the helpers
 * that open one on a given section. No components here, so any view can import it without
 * pulling the managers in.
 */

import { sectionKeyOf, type PaneSections } from "@/app/block/pane-sections";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";

export const CONNECTORS_VIEW = "connectors";
export const MEMORY_VIEW = "memory";
export const CONNECTORS_SECTION_KEY = sectionKeyOf(CONNECTORS_VIEW);
export const MEMORY_SECTION_KEY = sectionKeyOf(MEMORY_VIEW);
/** Block meta key for the agent Memory → Personal has open (an agent definition id). */
export const MEMORY_AGENT_KEY = "memory:agent";

export type ConnectorsSection = "accounts" | "mcp" | "remotes";
export type MemorySection = "global" | "personal" | "skills" | "bundles";

/** Connectors' sections, as its manifest declares them; the instance adds each one's component. */
export const CONNECTORS_SECTIONS: PaneSections<ConnectorsSection> = {
    default: "accounts",
    items: [
        { id: "accounts", label: "Accounts", icon: "key", tooltip: "Sign-ins to providers and services, bound to agents" },
        { id: "mcp", label: "MCP servers", icon: "plug" },
        { id: "remotes", label: "Remotes", icon: "server", tooltip: "Remote machines (SSH, WSL)" },
    ],
};

/** Memory's sections, as its manifest declares them; the instance adds each one's component. */
export const MEMORY_SECTIONS: PaneSections<MemorySection> = {
    default: "global",
    items: [
        { id: "global", label: "Global", icon: "globe", tooltip: "Instructions composed into every agent's startup file" },
        { id: "personal", label: "Personal", icon: "brain", tooltip: "What each agent writes in its own memory, with history" },
        { id: "skills", label: "Skills", icon: "wand-magic-sparkles" },
        {
            id: "bundles",
            label: "Bundles",
            icon: "layer-group",
            tooltip: "Bundles: instructions and skills to give any agent; imports Agent Bundle Format (ABF) files",
            highlight: true,
        },
    ],
};

/** Opens Connectors, or focuses the one already in this tab, on `section`. */
export function openConnectors(section: ConnectorsSection = "accounts"): Promise<void> {
    const meta = { [CONNECTORS_SECTION_KEY]: section };
    return openOrFocusPaneByView(CONNECTORS_VIEW, { meta: { view: CONNECTORS_VIEW, ...meta } }, meta);
}

/**
 * Opens Memory, or focuses the one already in this tab, on `section`. With
 * `agentId` it shows that agent's Personal memory (the section is "personal").
 */
export function openMemory(section: MemorySection = "global", opts?: { agentId?: string }): Promise<void> {
    const meta: Record<string, string> = { [MEMORY_SECTION_KEY]: section };
    if (opts?.agentId) meta[MEMORY_AGENT_KEY] = opts.agentId;
    return openOrFocusPaneByView(MEMORY_VIEW, { meta: { view: MEMORY_VIEW, ...meta } }, meta);
}
