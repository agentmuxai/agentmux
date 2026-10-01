// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Unloading collapsed tool results from the pane's memory: which tool nodes
 * may drop their `result`, and the small stub that stands in for it until
 * the row is opened and the result is read back from the transcript.
 * docs/specs/SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01.md §3.3.
 *
 * Pure: the pane decides when to run a pass and dispatches the result as one
 * `UnloadToolResults` reducer command.
 */

import { estimateTokenCount } from "@/util/format-count";
import { toolActivities } from "./activity/tool-adapter";
import { RETENTION_MS } from "./activity/types";
import { isContentFirstTool, toolPill } from "./tool-meta/tool-descriptors";
import type { DocumentNode, ToolNode, ToolResult } from "./types";

/**
 * Results smaller than this stay: they cost little, and small reads of a
 * result (background-launch detection, `isAcceptedBackgroundLaunch`) keep
 * working unchanged.
 */
export const UNLOAD_MIN_BYTES = 8 * 1024;

/** Slack past the dock's own retention, for a row still fading out. */
const DOCK_GRACE_MS = 5_000;

const FINISHED = new Set<ToolNode["status"]>(["success", "failed", "denied", "canceled"]);

/** A result's size as the pane holds it (its JSON text). */
export function resultBytes(result: ToolResult | undefined): number {
    if (result == null) return 0;
    try {
        return JSON.stringify(result).length;
    } catch {
        return 0;
    }
}

/**
 * Ids of the tool nodes whose results may unload now: finished, collapsed
 * (not in `keepIds`: pinned, held open, or otherwise kept), before the turn
 * in flight, with a known transcript line, at least UNLOAD_MIN_BYTES, not
 * a content-first tool (whose body shows while collapsed), and not still
 * showing in the Activity Dock, whose row reads its output from the result
 * (Codex P2 on #4126). A later pass unloads it once the dock lets it go.
 * Only results from a global (`g:`) stream unload: a pane that started on its
 * block's own `b:` file moves to the global zone once that has content, and
 * reads are then served from there, so a `b:` line can no longer be read back
 * (Codex P2 on #4126).
 */
export function planUnload(
    nodes: readonly DocumentNode[],
    opts: { keepIds: ReadonlySet<string>; now?: number }
): string[] {
    const now = opts.now ?? Date.now();
    const docked = new Set<string>();
    for (const a of toolActivities(nodes, now)) {
        if (a.endedAt == null || now - a.endedAt < RETENTION_MS[a.status] + DOCK_GRACE_MS) docked.add(a.id);
    }
    let lastUser = -1;
    for (let i = nodes.length - 1; i >= 0; i--) {
        if (nodes[i].type === "user_message") {
            lastUser = i;
            break;
        }
    }
    const ids: string[] = [];
    for (let i = 0; i < lastUser; i++) {
        const n = nodes[i];
        if (n.type !== "tool") continue;
        if (!FINISHED.has(n.status)) continue;
        if (n.result == null || n.resultUnloaded || !n.resultSource?.stream.startsWith("g:")) continue;
        if (opts.keepIds.has(n.id) || docked.has(n.id)) continue;
        if (isContentFirstTool(n)) continue;
        if (resultBytes(n.result) < UNLOAD_MIN_BYTES) continue;
        ids.push(n.id);
    }
    return ids;
}

/** The node with its result replaced by a stub: the pill and size stay. */
export function unloadResult(node: ToolNode): ToolNode {
    const bytes = resultBytes(node.result);
    const pill = toolPill(node);
    const text = JSON.stringify(node.result ?? "");
    return {
        ...node,
        result: undefined,
        resultUnloaded: { pill: pill ? { label: pill.label, variant: pill.variant } : null, bytes, tokens: estimateTokenCount(text) },
    };
}
