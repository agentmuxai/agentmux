// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The context-compaction cards the agent stream pushes into the document (moved out of useAgentStream.ts; re-exported there). */

import type { AgentPaneEvent } from "@/app/store/agent-pane-state/types";
import type { ContextCompactedNode } from "../../agent-model/types";
import { contextCompactedNodeId, contextCompactedLiveTimestamp } from "./compact-boundary";
import type { StreamFlushQueue } from "./stream-flush-queue";

/**
 * Shared node-construction for the reducer's `context-compacted` event —
 * used by BOTH the real `CompactionBoundary` path and the `TokensIn`
 * heuristic fallback, so the two never drift into different node shapes.
 * `source`/`trigger`/`durationMs` come straight from the event: "real"
 * carries all three, "heuristic" carries only `source`. See
 * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.3.
 */
export function pushContextCompactedNodes(
    paneEvents: AgentPaneEvent[] | undefined,
    queue: StreamFlushQueue,
    hasNodeId: (id: string) => boolean,
    addNodeId: (id: string) => void,
): ContextCompactedNode | null {
    // The last real card pushed: its `contextAfter` is filled in by the next
    // main-agent call (`fillCompactionCard`).
    let realCard: ContextCompactedNode | null = null;
    for (const ev of paneEvents ?? []) {
        if (ev.type !== "context-compacted") continue;
        // A "real" event (a genuine compact_boundary frame) is keyed and timestamped by the same
        // functions parseHistoryLines.ts uses (compact-boundary.ts), so the two cannot drift. The
        // heuristic path has no frame to key or time on, so it keeps a Date.now()-based id and
        // timestamp: history replay never produces a node for it to collide with.
        const id =
            ev.source === "real"
                ? contextCompactedNodeId({
                      trigger: ev.trigger,
                      preTokens: ev.tokensBefore,
                      postTokens: ev.tokensAfter,
                      durationMs: ev.durationMs,
                      uuid: ev.boundaryUuid,
                      frameTimestamp: ev.frameTimestamp,
                  })
                : `context-compacted-${Date.now()}`;
        const timestamp =
            ev.source === "real" ? contextCompactedLiveTimestamp(ev.frameTimestamp) : Date.now();
        const compactNode: ContextCompactedNode = {
            type: "context_compacted",
            id,
            tokensBefore: ev.tokensBefore,
            tokensAfter: ev.tokensAfter,
            timestamp,
            source: ev.source,
            trigger: ev.trigger,
            durationMs: ev.durationMs,
        };
        if (!hasNodeId(compactNode.id)) {
            addNodeId(compactNode.id);
            queue.pushNewNode(compactNode);
            queue.scheduleFlush();
            if (compactNode.source === "real") realCard = compactNode;
        }
    }
    return realCard;
}

/** `card` with the context's real size after it — the first main-agent call
 *  since the boundary — queued as an in-place update. */
export function fillCompactionCard(card: ContextCompactedNode, contextAfter: number, queue: StreamFlushQueue): void {
    queue.pushUpdatedNode({ ...card, contextAfter });
    queue.scheduleFlush();
}
