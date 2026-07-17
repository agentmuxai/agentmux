// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PinnedActivity — the unified abstraction behind the pinned activity dock.
 *
 * Anything long-running an agent spawns (a shell, a cron, a subagent) maps onto
 * this contract and renders as a uniform row in the dock at the top of the
 * agent pane. Phase 1 implemented the `shell` kind; Phase 2 (this file) adds
 * `subagent`. `cron` still has no adapter — it's sugar over a `shell` per the
 * spec (§6), not yet built.
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md
 */

import type { ShellNode } from "../types";
import type { ActiveSubagent } from "../../swarm/swarm-model";

export type ActivityKind = "shell" | "cron" | "subagent";

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
}

/** Per-kind sigil; colored by status in CSS. */
export const KIND_SIGIL: Record<ActivityKind, string> = {
    shell: "⟩",
    cron: "⟳",
    subagent: "◆",
};

/** Milliseconds a terminal row lingers in the dock before auto-dismiss (D4).
 *  `error` is Infinity — it persists until the user acknowledges it. */
export const RETENTION_MS: Record<ActivityStatus, number> = {
    running: Infinity,
    done: 8_000,
    stopped: 3_000,
    error: Infinity,
};
