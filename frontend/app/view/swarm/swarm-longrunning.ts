// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Long-running tool activity, per agent, for the Swarm pane.
 *
 * Swarm already answers "what is this agent doing" for the things srv owns —
 * subagent dispatches, Shell-MCP shells, crons. Ordinary Bash tool calls are
 * the gap: a `sleep 300` or a 4-minute build shows in that pane's own Activity
 * Dock and nowhere else, so the fleet view can't answer "which agents are
 * sitting on a wait right now" without opening each one.
 *
 * This is step 4 of `REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md`
 * §3 — "feed the same signal to Swarm ... same data, same adapter output, new
 * renderer".
 *
 * ## Why this reads the document store directly
 *
 * The other three buckets come from srv because shells, crons and subagents are
 * srv-owned objects. A Bash tool call is not: it exists only in the CLI's
 * transcript stream, and the classification of "long-running" lives entirely in
 * `tool-adapter.ts` (srv sees raw `ToolNode` statuses via `DockNodeStatusCommand`
 * but explicitly does NOT model the dock's promotion rules — see
 * `dock_snapshot.rs`'s own doc comment saying that reclassification "happens
 * entirely client-side ... which the server has no visibility into").
 *
 * Rather than duplicate that classifier in Rust, this reuses it: `slots` in
 * `agent-document-store.ts` is a module-level map keyed by block id, and Swarm
 * runs in the same renderer process, so `toolActivities` can be applied to any
 * pane's nodes directly. One classifier, every consumer reuses it — the same
 * discipline `isAcceptedBackgroundLaunch` established.
 *
 * ## Known limitation, stated rather than hidden
 *
 * `snapshot()` only returns state for a pane that is currently REGISTERED —
 * `registerPane` on mount, `unregisterPane` on unmount. An agent whose pane
 * isn't mounted contributes nothing here, and reads as zero rather than
 * unknown. That's the same population that has a live dock at all, so the two
 * views agree; it is NOT full fleet coverage of unopened panes. Closing that
 * would need srv to model the promotion rules, which is a much larger change
 * than this bucket justifies.
 */

import { snapshot } from "@/app/store/agent-document-store";
import { toolActivities } from "@/app/view/agent/activity/tool-adapter";
import type { PinnedActivity } from "@/app/view/agent/activity/types";

/** One long-running tool call, as Swarm needs to render it. */
export interface LongRunningToolRow {
    id: string;
    /** The command text (dock row title), e.g. `sleep 300`. */
    title: string;
    startedAt: number;
    /** Set only for a whole-command sleep — enables a real countdown. */
    sleepMs?: number;
}

/**
 * The RUNNING long-running tool calls attached to `blockId`, newest first —
 * matching the other buckets' ordering convention.
 *
 * Running only: a finished call lingers in `toolActivities` through the dock's
 * retention window so its row can resolve in place, but Swarm is answering
 * "what is this agent on right now", where a completed call is noise.
 *
 * Returns `[]` for an unmounted pane (see the module doc's limitation note) and
 * for a block id that never existed, so callers need no special-casing.
 */
export function longRunningToolRows(blockId: string | null, now: number): LongRunningToolRow[] {
    if (!blockId) return [];
    const state = snapshot(blockId);
    if (!state) return [];
    return toolActivities(state.nodes, now)
        .filter((a: PinnedActivity) => a.status === "running")
        .map((a) => ({
            id: a.id,
            title: a.title,
            startedAt: a.startedAt,
            ...(a.sleepMs != null ? { sleepMs: a.sleepMs } : {}),
        }))
        .sort((a, b) => b.startedAt - a.startedAt);
}
