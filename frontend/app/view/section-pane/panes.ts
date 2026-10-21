// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Connectors and Memory panes: their view ids, section ids and meta keys, and the helpers
 * that open one on a given section. No components here, so any view can import it without
 * pulling the managers in.
 */

import { sectionKeyOf } from "@/app/block/pane-sections";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";

export const CONNECTORS_VIEW = "connectors";
export const MEMORY_VIEW = "memory";
export const CONNECTORS_SECTION_KEY = sectionKeyOf(CONNECTORS_VIEW);
export const MEMORY_SECTION_KEY = sectionKeyOf(MEMORY_VIEW);

export const CONNECTORS_SECTION_IDS = ["accounts", "mcp", "remotes"] as const;
export const MEMORY_SECTION_IDS = ["global", "personal", "skills", "bundles"] as const;
export type ConnectorsSection = (typeof CONNECTORS_SECTION_IDS)[number];
export type MemorySection = (typeof MEMORY_SECTION_IDS)[number];

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
