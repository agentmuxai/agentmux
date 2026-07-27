// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tool adapter — promotes an in-flight Bash `ToolNode` to a `PinnedActivity`
 * once it has been running past `TOOL_PROMOTION_MS`, mirroring
 * shell-adapter.ts's pattern. Unlike shells (an explicit, agent-opt-in MCP
 * tool already tracked end-to-end), an ordinary Bash tool call is otherwise
 * completely invisible to the dock — this is what closes that gap.
 *
 * Classification is duration-first, not text-pattern-first: any Bash call
 * still running past the threshold promotes, regardless of what its command
 * text looks like. This sidesteps false-positive risk from trying to
 * pattern-match `sleep`/`wait`/etc against arbitrary shell one-liners.
 * TOOL_PROMOTION_MS reuses the figure from
 * SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md §5's "overrun promotion".
 *
 * See docs/specs/REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md
 * §3 step 1, §4.1-4.3.
 */

import { extractToolDetail } from "../stream-parser";
import type { DocumentNode, ToolNode } from "../types";
import type { PinnedActivity } from "./types";

export const TOOL_PROMOTION_MS = 30_000;

function isPromotableBash(n: DocumentNode): n is ToolNode {
    return n.type === "tool" && n.tool === "Bash" && n.status === "running" && n.timestamp != null;
}

export function toolToActivity(n: ToolNode): PinnedActivity {
    const detail = extractToolDetail(n.tool, (n.params as Record<string, any>) ?? {});
    return {
        id: n.id,
        kind: "tool",
        title: detail || n.toolName || n.tool,
        status: "running",
        startedAt: n.timestamp!,
        // No cancel path exists for a single in-flight tool call today (only
        // a whole-turn interrupt) — same reasoning as subagent-adapter.ts's
        // canStop: false.
        canStop: false,
        tool: n,
    };
}

/** Bash tool calls still running past `TOOL_PROMOTION_MS` as of `now`. */
export function toolActivities(nodes: ReadonlyArray<DocumentNode>, now: number): PinnedActivity[] {
    const out: PinnedActivity[] = [];
    for (const n of nodes) {
        if (!isPromotableBash(n)) continue;
        if (now - n.timestamp! < TOOL_PROMOTION_MS) continue;
        out.push(toolToActivity(n));
    }
    return out;
}

/**
 * Earliest wall-clock time at which some currently-running, not-yet-promoted
 * Bash call will cross the promotion threshold — or null if none are
 * pending. Lets a caller schedule a single one-shot timer for exactly when
 * the next promotion should happen, instead of a continuous per-second tick
 * (see ActivityDock.tsx's `hasExpiring` for the same discipline applied to
 * retention expiry).
 */
export function nextToolPromotionAt(nodes: ReadonlyArray<DocumentNode>, now: number): number | null {
    let next: number | null = null;
    for (const n of nodes) {
        if (!isPromotableBash(n)) continue;
        const promotesAt = n.timestamp! + TOOL_PROMOTION_MS;
        if (promotesAt <= now) continue;
        if (next == null || promotesAt < next) next = promotesAt;
    }
    return next;
}

/** True if `id` (a ToolNode id) currently has a live dock entry — used by
 *  AgentWorkingRow to go calm/neutral once the dock has taken over showing
 *  this tool call's progress (report §4.3). */
export function isToolPromoted(nodes: ReadonlyArray<DocumentNode>, id: string | null, now: number): boolean {
    if (!id) return false;
    for (const n of nodes) {
        if (n.type === "tool" && n.id === id) {
            return isPromotableBash(n) && now - n.timestamp! >= TOOL_PROMOTION_MS;
        }
    }
    return false;
}
