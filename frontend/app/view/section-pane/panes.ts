// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Connectors and Memory panes: their view ids, section ids and meta keys, and the helpers
 * that open one on a given section. No components here, so any view can import it without
 * pulling the managers in.
 */

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";

export const CONNECTORS_VIEW = "connectors";
export const MEMORY_VIEW = "memory";
export const CONNECTORS_SECTION_KEY = "connectors:section";
export const MEMORY_SECTION_KEY = "memory:section";

export type ConnectorsSection = "accounts" | "mcp" | "remotes";
export type MemorySection = "global" | "personal" | "skills" | "bundles";

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
