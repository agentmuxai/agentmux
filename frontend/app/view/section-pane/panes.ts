// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Connectors and Knowledge panes that replaced the Armory
 * (docs/specs/SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md):
 * their view ids, section ids and meta keys, the helpers that open one on a
 * given section, and the mapping that moves a saved Armory pane to its new home.
 * No components here, so any view can import it without pulling the managers in.
 */

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";

export const CONNECTORS_VIEW = "connectors";
export const KNOWLEDGE_VIEW = "knowledge";
export const CONNECTORS_SECTION_KEY = "connectors:section";
export const KNOWLEDGE_SECTION_KEY = "knowledge:section";

export type ConnectorsSection = "accounts" | "mcp";
export type KnowledgeSection = "global" | "personal" | "skills" | "bundles";

export const CONNECTORS_SECTIONS: readonly ConnectorsSection[] = ["accounts", "mcp"];
export const KNOWLEDGE_SECTIONS: readonly KnowledgeSection[] = ["global", "personal", "skills", "bundles"];

/** Opens Connectors, or focuses the one already in this tab, on `section`. */
export function openConnectors(section: ConnectorsSection = "accounts"): Promise<void> {
    const meta = { [CONNECTORS_SECTION_KEY]: section };
    return openOrFocusPaneByView(CONNECTORS_VIEW, { meta: { view: CONNECTORS_VIEW, ...meta } }, meta);
}

/** Opens Knowledge, or focuses the one already in this tab, on `section`. */
export function openKnowledge(section: KnowledgeSection = "global"): Promise<void> {
    const meta = { [KNOWLEDGE_SECTION_KEY]: section };
    return openOrFocusPaneByView(KNOWLEDGE_VIEW, { meta: { view: KNOWLEDGE_VIEW, ...meta } }, meta);
}

/** Where a saved Armory pane goes, from its `armory:section` (and, for the old
 *  Memory section, `armory:memory:subsection`). Unknown or unset sections go to
 *  Connectors → Accounts, which is where the Armory itself opened. Mirrors
 *  `armory_target` in crates/srv/src/backend/layout_file.rs. */
export function armoryTarget(meta: Record<string, unknown> | null | undefined): {
    view: string;
    key: string;
    section: ConnectorsSection | KnowledgeSection;
} {
    const section = meta?.["armory:section"];
    const sub = meta?.["armory:memory:subsection"];
    const knowledge = (s: KnowledgeSection) => ({ view: KNOWLEDGE_VIEW, key: KNOWLEDGE_SECTION_KEY, section: s });
    switch (section) {
        case "mcp":
            return { view: CONNECTORS_VIEW, key: CONNECTORS_SECTION_KEY, section: "mcp" };
        case "memory":
            return knowledge(sub === "personal" ? "personal" : "global");
        case "native_memory":
            return knowledge("personal");
        case "skills":
            return knowledge("skills");
        case "bundles":
            return knowledge("bundles");
        default:
            return { view: CONNECTORS_VIEW, key: CONNECTORS_SECTION_KEY, section: "accounts" };
    }
}

/** The meta patch that turns a saved Armory block into its new pane. Keeps
 *  everything else (term:zoom), drops the Armory's own keys. */
export function armoryMigrationPatch(meta: Record<string, unknown> | null | undefined): Record<string, unknown> {
    const t = armoryTarget(meta);
    return { view: t.view, [t.key]: t.section, "armory:section": null, "armory:memory:subsection": null };
}
