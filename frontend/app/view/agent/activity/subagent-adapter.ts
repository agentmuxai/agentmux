// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Subagent adapter — maps `ActiveSubagent`s (swarm-model.ts, sourced from the
 * shared `subagent-source.ts` singleton) onto `PinnedActivity` for the dock,
 * filtered to one agent pane's own spawns (`parent_block_id`). Phase 2 of the
 * dock — proves the `PinnedActivity` abstraction generalizes beyond shells.
 *
 * `canStop` is always `false`: no subagent-cancel RPC/UI exists anywhere in
 * the app today (confirmed — `AgentStopCommand` targets a pane's own agent
 * process, not a subagent by `agent_id`; the Swarm pane itself has no cancel
 * action either). Wiring a real cancel is its own follow-up, not invented
 * here just because the dock's row chrome has a stop button slot.
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md (§3, §7)
 */

import type { ActiveSubagent } from "../../swarm/swarm-model";
import type { ActivityStatus, PinnedActivity } from "./types";

function subagentStatusToActivity(s: ActiveSubagent["status"]): ActivityStatus {
    switch (s) {
        case "active": return "running";
        case "completed": return "done";
        // Terminated without a Result line (parent turn ended early) — same
        // bucket as a shell's manual "stopped", not "error" (no failure
        // signal, just cut off). See SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION.
        case "abandoned": return "stopped";
    }
}

export function subagentToActivity(s: ActiveSubagent): PinnedActivity {
    return {
        id: s.agent_id,
        kind: "subagent",
        // `||`, not `??` — an empty-string slug (before the watcher reads the
        // JSONL's first line) must fall through to agent_id, same as
        // swarm-view.tsx's own equivalent fallback chain.
        title: s.display_name || s.slug || s.agent_id,
        status: subagentStatusToActivity(s.status),
        startedAt: s.spawned_at,
        // last_event_at at the moment a subagent completes (or is reconciled
        // to abandoned) is the closest thing to an "ended at" ActiveSubagent
        // exposes (there is no dedicated field). Both are terminal — leaving
        // abandoned out would tick the elapsed timer forever and never let
        // D4 retention (RETENTION_MS.stopped) auto-dismiss the row.
        endedAt: s.status === "completed" || s.status === "abandoned" ? s.last_event_at : undefined,
        canStop: false,
        subagent: s,
    };
}

/** This pane's own subagents (`parent_block_id === blockId`), mapped to activities. */
export function subagentActivities(all: ReadonlyArray<ActiveSubagent>, blockId: string): PinnedActivity[] {
    const out: PinnedActivity[] = [];
    for (const s of all) {
        if (s.parent_block_id === blockId) out.push(subagentToActivity(s));
    }
    return out;
}
