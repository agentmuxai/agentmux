// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Connectors and Memory panes that replaced the Armory
 * (docs/specs/SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md;
 * Memory was named Knowledge until SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md):
 * their view ids, section ids and meta keys, the helpers that open one on a
 * given section, and the mapping that moves a saved Armory pane to its new home.
 * No components here, so any view can import it without pulling the managers in.
 */

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";

export const CONNECTORS_VIEW = "connectors";
export const MEMORY_VIEW = "memory";
export const CONNECTORS_SECTION_KEY = "connectors:section";
export const MEMORY_SECTION_KEY = "memory:section";
/** Memory's former view id and section key, still found in saved blocks and
 *  layout files: the view resolves as an alias, the key is read as a fallback. */
export const LEGACY_KNOWLEDGE_VIEW = "knowledge";
export const LEGACY_KNOWLEDGE_SECTION_KEY = "knowledge:section";

export type ConnectorsSection = "accounts" | "mcp";
export type MemorySection = "global" | "personal" | "skills" | "bundles";

export const CONNECTORS_SECTIONS: readonly ConnectorsSection[] = ["accounts", "mcp"];
export const MEMORY_SECTIONS: readonly MemorySection[] = ["global", "personal", "skills", "bundles"];

/** Opens Connectors, or focuses the one already in this tab, on `section`. */
export function openConnectors(section: ConnectorsSection = "accounts"): Promise<void> {
    const meta = { [CONNECTORS_SECTION_KEY]: section };
    return openOrFocusPaneByView(CONNECTORS_VIEW, { meta: { view: CONNECTORS_VIEW, ...meta } }, meta);
}

/** Opens Memory, or focuses the one already in this tab, on `section`. */
export function openMemory(section: MemorySection = "global"): Promise<void> {
    const meta = { [MEMORY_SECTION_KEY]: section };
    return openOrFocusPaneByView(MEMORY_VIEW, { meta: { view: MEMORY_VIEW, ...meta } }, meta);
}

/** Where a saved Armory pane goes, from its `armory:section` (and, for the old
 *  Memory section, `armory:memory:subsection`). Unknown or unset sections go to
 *  Connectors → Accounts, which is where the Armory itself opened. Mirrors
 *  `armory_target` in crates/srv/src/backend/layout_file.rs. */
export function armoryTarget(meta: Record<string, unknown> | null | undefined): {
    view: string;
    key: string;
    section: ConnectorsSection | MemorySection;
} {
    const section = meta?.["armory:section"];
    const sub = meta?.["armory:memory:subsection"];
    const memory = (s: MemorySection) => ({ view: MEMORY_VIEW, key: MEMORY_SECTION_KEY, section: s });
    switch (section) {
        case "mcp":
            return { view: CONNECTORS_VIEW, key: CONNECTORS_SECTION_KEY, section: "mcp" };
        case "memory":
            return memory(sub === "personal" ? "personal" : "global");
        case "native_memory":
            return memory("personal");
        case "skills":
            return memory("skills");
        case "bundles":
            return memory("bundles");
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
