// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The agent colour on a Kanban card (SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md §7.2): the
// colour its pane and Swarm row show. From the agent's pane when it has one here (the operator's pick in
// the Pane Color menu included), else from the colour srv keeps for it, through the same theme-tuned
// role mapping; none when the task names no known agent. Never a hash of the name: that would show a
// colour the agent doesn't have.

import type { PaneColorRole } from "@/app/block/pane-color-scheme";
import { blockRoleColor, isLightThemeActive } from "@/app/block/pane-identity";
import { MOS } from "@/app/store/global";

import type { KanbanAgent } from "./kanban-types";

/** A block's meta, read reactively. */
function blockMeta(blockId: string): Block["meta"] | undefined {
    return MOS.getMuxObjectAtom<Block>(`block:${blockId}`)()?.meta;
}

/** An agent's colour for `role` in the current theme, or undefined when it has none. */
export function agentColor(
    agent: KanbanAgent | null | undefined,
    uid: string | undefined,
    colors: Record<string, string> | undefined,
    role: PaneColorRole = "border"
): string | undefined {
    const light = isLightThemeActive();
    if (agent?.block_id) {
        const fromPane = blockRoleColor(blockMeta(agent.block_id), light, role, { widget: false });
        if (fromPane) return fromPane;
    }
    const hex = agent?.color || (uid ? colors?.[uid] : undefined);
    if (!hex) return undefined;
    return blockRoleColor({ "frame:activebordercolor": hex } as Block["meta"], light, role, { widget: false });
}
