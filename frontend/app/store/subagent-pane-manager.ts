// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase 3: On-demand subagent pane creation.
// The agent pane renders clickable subagent links when subagents are detected.
// Clicking a link calls openSubagentPane() to split off a subagent activity view.

import { createBlockSplitHorizontally, createBlockSplitVertically } from "./global";

// Track open subagent panes: subagentId -> { blockId, parentBlockId }
const openPanes = new Map<string, { blockId: string; parentBlockId: string }>();
// Track how many subagent panes are open for a given parent block
const parentPaneCount = new Map<string, number>();

export interface SubagentPaneRequest {
    subagentId: string;
    slug: string;
    parentAgent: string;
    parentBlockId: string;
    sessionId: string;
}

/**
 * Open a subagent activity pane, split from the parent agent's block.
 * Called when the user clicks a subagent link in the agent pane.
 * Returns the new block ID, or null if the pane is already open.
 */
export async function openSubagentPane(req: SubagentPaneRequest): Promise<string | null> {
    // Don't open duplicate panes
    if (openPanes.has(req.subagentId)) {
        console.log(
            `[subagent-pane] pane already open for ${req.subagentId}`
        );
        return openPanes.get(req.subagentId)?.blockId ?? null;
    }

    const blockDef: BlockDef = {
        meta: {
            view: "subagent",
            "subagent:id": req.subagentId,
            "subagent:slug": req.slug,
            "subagent:parent": req.parentAgent,
            "subagent:session": req.sessionId,
        },
    };

    const count = parentPaneCount.get(req.parentBlockId) ?? 0;
    let newBlockId: string;

    try {
        if (count === 0) {
            // First subagent pane: split horizontally (parent left, subagent right)
            newBlockId = await createBlockSplitHorizontally(
                blockDef,
                req.parentBlockId,
                "after"
            );
        } else {
            // Subsequent: find an existing open subagent pane and stack vertically
            const siblingBlockId = findOpenSiblingPane(req.parentBlockId);
            if (siblingBlockId) {
                newBlockId = await createBlockSplitVertically(
                    blockDef,
                    siblingBlockId,
                    "after"
                );
            } else {
                newBlockId = await createBlockSplitHorizontally(
                    blockDef,
                    req.parentBlockId,
                    "after"
                );
            }
        }
    } catch (err) {
        console.error("[subagent-pane] split failed:", err);
        return null;
    }

    openPanes.set(req.subagentId, { blockId: newBlockId, parentBlockId: req.parentBlockId });
    parentPaneCount.set(req.parentBlockId, count + 1);

    console.log(
        `[subagent-pane] opened pane ${newBlockId} for subagent ${req.subagentId} (parent: ${req.parentAgent})`
    );
    return newBlockId;
}

/**
 * Check if a subagent pane is already open.
 */
export function isSubagentPaneOpen(subagentId: string): boolean {
    return openPanes.has(subagentId);
}

/**
 * Notify the manager that a subagent pane was closed (by the user or layout).
 */
export function onSubagentPaneClosed(subagentId: string, parentBlockId: string): void {
    openPanes.delete(subagentId);
    const count = parentPaneCount.get(parentBlockId) ?? 0;
    if (count > 0) {
        parentPaneCount.set(parentBlockId, count - 1);
    }
}

/**
 * Find an existing open subagent pane belonging to the same parent to stack next to.
 */
function findOpenSiblingPane(parentBlockId: string): string | null {
    for (const [, entry] of openPanes) {
        if (entry.parentBlockId === parentBlockId) {
            return entry.blockId;
        }
    }
    return null;
}
