// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Which layout an Atlas pane shows, kept with the pane (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md decision 1). */
export type AtlasLayoutMode = "machine" | "seat";

export const ATLAS_LAYOUT_META = "atlas:layout";

export function readAtlasLayout(meta: Record<string, unknown> | undefined): AtlasLayoutMode {
    return meta?.[ATLAS_LAYOUT_META] === "seat" ? "seat" : "machine";
}
