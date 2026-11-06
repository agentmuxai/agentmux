// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where "Open it" takes an agent that runs but that its picker finds in no open window. A parked agent (its
 * block carries `agent:parked`) has no pane: it opens one in the active tab. An agent with a block that is
 * not parked has a pane, in a workspace no window shows now (closing a workspace leaves its panes): reveal
 * that pane, which brings its workspace back, as a click in the Swarm does, instead of opening a second pane.
 */

import { isParkedMeta } from "@/app/view/swarm/swarm-open-parked";

/** The block to reveal, or null when the agent has no pane to reveal. */
export function paneToRevealFor(
    runtime: { block_id?: string } | undefined,
    blockMeta: Record<string, unknown> | undefined
): string | null {
    const blockId = runtime?.block_id;
    if (!blockId || isParkedMeta(blockMeta)) return null;
    return blockId;
}
