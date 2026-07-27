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
 * Once promoted, a call keeps its dock row through the normal done/error
 * retention window instead of vanishing the instant ToolEnd lands — same
 * "linger forever in the adapter's output, let the dock's own RETENTION_MS
 * filtering decide visibility" shape shell-adapter.ts uses (see
 * ActivityDock.tsx's `visible()` and its comment on `hasExpiring`).
 * `ToolNode` has no explicit "ended at" field, so a finished call's
 * `endedAt` is derived from `timestamp + duration` (duration is only ever
 * populated once the call is terminal).
 *
 * See docs/specs/REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md
 * §3 step 1, §4.1-4.3.
 */

import { extractToolDetail } from "../stream-parser";
import type { DocumentNode, ToolNode } from "../types";
import type { ActivityStatus, PinnedActivity } from "./types";

export const TOOL_PROMOTION_MS = 30_000;

function isBashToolNode(n: DocumentNode): n is ToolNode {
    return n.type === "tool" && n.tool === "Bash" && n.timestamp != null;
}

/** True while `n` is running and has already crossed the promotion
 *  threshold as of `now` — the "should suppress AgentWorkingRow / should
 *  schedule a promotion timer" condition. Deliberately narrower than
 *  `everCrossedThreshold` below: it must NOT go true for a call that has
 *  already finished (that case is handled entirely by its retained,
 *  terminal-status dock row, not by anything time-gated). */
function isRunningPastThreshold(n: ToolNode, now: number): boolean {
    return n.status === "running" && now - n.timestamp! >= TOOL_PROMOTION_MS;
}

/** True once `n` is known to have run for at least `TOOL_PROMOTION_MS`,
 *  whether it's still running now or has already finished. A call that
 *  finished in under the threshold never appears in the dock at all — this
 *  is the "did this call ever deserve a dock row" gate for `toolActivities`. */
function everCrossedThreshold(n: ToolNode, now: number): boolean {
    if (n.status === "running") return isRunningPastThreshold(n, now);
    if (n.duration != null) return n.duration * 1000 >= TOOL_PROMOTION_MS;
    return false;
}

function toolActivityStatus(status: ToolNode["status"]): ActivityStatus {
    switch (status) {
        case "running": return "running";
        case "success": return "done";
        case "failed": return "error";
        // denied/canceled/pending_approval/awaiting_answer — cut off or
        // never actually ran long, not a failure signal. Same bucket as
        // subagent-adapter.ts's "abandoned" → "stopped".
        default: return "stopped";
    }
}

export function toolToActivity(n: ToolNode): PinnedActivity {
    const detail = extractToolDetail(n.tool, (n.params as Record<string, any>) ?? {});
    return {
        id: n.id,
        kind: "tool",
        title: detail || n.toolName || n.tool,
        status: toolActivityStatus(n.status),
        startedAt: n.timestamp!,
        endedAt: n.status !== "running" && n.duration != null ? n.timestamp! + n.duration * 1000 : undefined,
        // No cancel path exists for a single in-flight tool call today (only
        // a whole-turn interrupt) — same reasoning as subagent-adapter.ts's
        // canStop: false.
        canStop: false,
        tool: n,
    };
}

/** Every Bash tool call that has ever crossed `TOOL_PROMOTION_MS` — running
 *  ones past the threshold, plus finished ones whose total duration cleared
 *  it (still returned after they finish so the dock's own RETENTION_MS
 *  window, not this function, decides when the row actually disappears). */
export function toolActivities(nodes: ReadonlyArray<DocumentNode>, now: number): PinnedActivity[] {
    const out: PinnedActivity[] = [];
    for (const n of nodes) {
        if (!isBashToolNode(n)) continue;
        if (!everCrossedThreshold(n, now)) continue;
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
 * retention expiry). Scoped to running calls only — a finished call's dock
 * row is decided synchronously from its final `duration`, nothing to
 * schedule for it.
 */
export function nextToolPromotionAt(nodes: ReadonlyArray<DocumentNode>, now: number): number | null {
    let next: number | null = null;
    for (const n of nodes) {
        if (!isBashToolNode(n) || n.status !== "running") continue;
        const promotesAt = n.timestamp! + TOOL_PROMOTION_MS;
        if (promotesAt <= now) continue;
        if (next == null || promotesAt < next) next = promotesAt;
    }
    return next;
}

/**
 * True if the pane currently has a live, already-promoted Bash call —
 * used by AgentWorkingRow to suppress its own "tool · arg" text once the
 * dock is already showing it (report §4.3). Deliberately narrower than
 * `toolActivities`: a *finished* call still lingering in the dock during
 * its retention window must not suppress a different, newly-started tool
 * call's own working-row text.
 */
export function hasRunningPromotedTool(nodes: ReadonlyArray<DocumentNode>, now: number): boolean {
    for (const n of nodes) {
        if (isBashToolNode(n) && isRunningPastThreshold(n, now)) return true;
    }
    return false;
}
