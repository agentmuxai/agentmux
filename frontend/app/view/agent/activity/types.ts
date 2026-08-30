// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PinnedActivity — the unified abstraction behind the pinned activity dock.
 *
 * Anything long-running an agent spawns (a shell, a cron, a subagent, an
 * overrunning tool call) maps onto this contract and renders as a uniform row
 * in the dock at the top of the agent pane. Phase 1 implemented the `shell`
 * kind; Phase 2 added `subagent`. `cron` still has no adapter — it's sugar
 * over a `shell` per the spec (§6), not yet built. `tool` (this file) closes
 * the gap where an ordinary Bash tool call (a `sleep`, a backgrounded dev
 * server) is otherwise invisible to the dock — see
 * docs/specs/REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md §3.
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md
 */

import type { ShellNode, ToolNode } from "../types";
import type { ActiveSubagent } from "../../swarm/swarm-model";

export type ActivityKind = "shell" | "cron" | "subagent" | "tool";

/** Normalized lifecycle across every kind (D3/D4 ordering + retention). */
export type ActivityStatus = "running" | "done" | "error" | "stopped";

export interface PinnedActivity {
    id: string;
    kind: ActivityKind;
    title: string;
    status: ActivityStatus;
    /** Unix ms — drives the elapsed timer and the D3 newest-first ordering. */
    startedAt: number;
    /** Unix ms when it reached a terminal status (drives D4 retention). */
    endedAt?: number;
    /** True while the activity can be stopped (running). */
    canStop: boolean;

    // ── Kind-specific source, read by the row's tail + Expanded view ──
    /** Present when `kind === "shell"` (also "cron", which is a shell). */
    shell?: ShellNode;
    /** Present when `kind === "subagent"` AND it represents exactly one
     *  standalone subagent (no shared `workflow_id`/`display_name` group). */
    subagent?: ActiveSubagent;
    /** Present when `kind === "subagent"` AND it represents a GROUP — every
     *  subagent spawned together by one Task/Workflow-tool run, or every
     *  loose subagent sharing one Haiku-generated `display_name` — collapsed
     *  into a single dock row instead of one row per member. A single Agent
     *  tool call can spawn dozens of subagents at once; without this a dock
     *  row appeared per subagent instead of per tool call. Mutually
     *  exclusive with `subagent` above. */
    subagentGroup?: { members: ActiveSubagent[] };
    /** Present when `kind === "tool"` — an ordinary Bash tool call promoted
     *  after running past `TOOL_PROMOTION_MS` (tool-adapter.ts). */
    tool?: ToolNode;
    /** Present only when the command is a whole-command sleep
     *  (`activity/sleep-detect.ts`): how long it will wait, in ms. The one
     *  case where remaining time is genuinely KNOWN rather than estimated, so
     *  the row renders a countdown instead of a blind elapsed timer. Absent
     *  for every other activity — `sleep 90; tail log` deliberately does NOT
     *  get one, since the trailing work makes the total unknowable. */
    sleepMs?: number;
}

/** Per-kind sigil; colored by status in CSS. */
export const KIND_SIGIL: Record<ActivityKind, string> = {
    shell: "⟩",
    cron: "⟳",
    subagent: "◆",
    tool: "$",
};

/** Milliseconds a terminal row lingers in the dock before auto-dismiss (D4).
 *  `error` used to be Infinity (persist until the user manually dismissed it)
 *  — in practice this let failed background shells rack up in the dock
 *  indefinitely across a long session. 15s gives the user time to actually
 *  read the error (well above `done`/`stopped`) while still self-clearing. */
export const RETENTION_MS: Record<ActivityStatus, number> = {
    running: Infinity,
    done: 8_000,
    stopped: 3_000,
    error: 15_000,
};

/** Duration of the landing/departure flash (ActivityRow) — matches the tab
 *  landing bounce's own clear-timeout (tab-reorder.ts / tab-tearoff-events.ts)
 *  so a row visually reads the same way a dropped tab does. The dock's own
 *  visibility window is extended by this much past RETENTION_MS so the exit
 *  flash has time to actually play before the row leaves the DOM. */
export const EXIT_FLASH_MS = 400;
