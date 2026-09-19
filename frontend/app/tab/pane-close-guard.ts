// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane-level close confirmation — docs/specs/SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md §4.6.
 *
 * Closing a pane stops every agent in it (a turn in progress is interrupted,
 * tracked processes die with the agent). Before that happens, ask once for the
 * whole pane — listing the agents it would cut off — and only when something
 * is actually running. Idle panes close without a prompt.
 *
 * This replaces `useAgentCloseConfirm`, which patched `onClose` on the agent
 * view's own copy of the NodeModel; the pane header's × reads the leaf's
 * NodeModel instead, so that prompt never fired (spec §3.5).
 */

export interface BusyMember {
    blockId: string;
    name: string;
    turnActive: boolean;
    processCount: number;
}

/** How to look a block up. Injected so the logic is testable without RPC. */
export interface PaneCloseProbe {
    /** `turn_active` from the block's controller status; null if unknown. */
    turnActive(blockId: string): Promise<boolean | null>;
    /** Number of tracked OS processes the block's agent started. */
    processCount(blockId: string): Promise<number>;
    /** Display name for the prompt. */
    name(blockId: string): string;
}

/**
 * Members that would lose work if the pane closed now. A probe that fails
 * counts as "not busy": a broken status lookup must never make a pane
 * impossible to close.
 */
export async function busyMembers(blockIds: string[], probe: PaneCloseProbe): Promise<BusyMember[]> {
    const results = await Promise.all(
        blockIds.map(async (blockId) => {
            const [turnActive, processCount] = await Promise.all([
                probe.turnActive(blockId).catch(() => null),
                probe.processCount(blockId).catch(() => 0),
            ]);
            return { blockId, name: probe.name(blockId), turnActive: turnActive === true, processCount };
        })
    );
    return results.filter((m) => m.turnActive || m.processCount > 0);
}

/** One line per busy agent, e.g. "Posa — mid-turn, 2 processes running". */
export function describeBusyMember(m: BusyMember): string {
    const parts: string[] = [];
    if (m.turnActive) parts.push("mid-turn");
    if (m.processCount > 0) parts.push(`${m.processCount} ${m.processCount === 1 ? "process" : "processes"} running`);
    return `${m.name} — ${parts.join(", ")}`;
}
