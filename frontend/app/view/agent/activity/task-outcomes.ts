// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Background-task outcomes read from the agent's own persisted stream.
 *
 * The Claude CLI reports a background task's end on stdout as
 * `system/task_notification` (and `system/task_updated` with a terminal
 * `patch.status`). That line is in the durable transcript. It was used by
 * srv's live feed (`background_task_feed.rs` → the registry) but not by the
 * renderer: the dock closed an accepted background launch only from a
 * `<task-notification>` USER message — which the stream never carries — or from
 * the registry. The registry is per-instance and srv fills it only from LIVE
 * lines, so a pane opened on a fresh instance (an upgrade, a new channel, a
 * restart with an empty DB) replayed its history into an empty registry and
 * every finished background launch in the replay window stayed `running`
 * forever (2026-09-30: two rows for ~11.5 h).
 *
 * This reads the same facts from the same lines in both the live path
 * (`useAgentStream.ts`) and the replay path (`parseHistoryLines.ts`), so the
 * answer no longer depends on which srv happened to be watching.
 *
 * Only explicit end frames count. They are order-independent facts (a task
 * that ended stays ended), which matters because history pages load newest
 * first, so a frame can arrive before the `task_started` that names its
 * `tool_use_id`. Inferring an end from a task leaving `background_tasks_changed`
 * is deliberately NOT done here: that needs stream order, which paging breaks,
 * and a wrong guess would end a task that is genuinely still running.
 */

import { createSignal, type Accessor } from "solid-js";
import type { ActivityStatus, PinnedActivity } from "./types";

export interface TaskOutcome {
    status: ActivityStatus;
    /** Unix ms the end was observed, when the frame had a usable stamp. */
    endedAt?: number;
}

/** Same lenient mapping as tool-adapter's `parseTaskNotification` and srv's
 *  `terminal_status`: an unrecognised status still ENDS the task. */
function terminalStatus(raw: unknown): ActivityStatus {
    return raw === "completed" ? "done" : raw === "failed" ? "error" : "stopped";
}

const TERMINAL_UPDATE_STATUSES = new Set(["completed", "failed", "killed", "stopped"]);

function str(v: unknown): string | undefined {
    return typeof v === "string" && v !== "" ? v : undefined;
}

export class TaskOutcomeTracker {
    private readonly toolByTask = new Map<string, string>();
    /** Ends whose `tool_use_id` isn't known yet (`task_updated` carries only
     *  the task id). Resolved when the `task_started` arrives. */
    private readonly pendingByTask = new Map<string, TaskOutcome>();
    private readonly byTool = new Map<string, TaskOutcome>();

    get outcomes(): ReadonlyMap<string, TaskOutcome> {
        return this.byTool;
    }

    /** Feed one parsed stdout frame. Returns true when an outcome was added or
     *  changed, so a caller can skip publishing an identical snapshot. */
    note(frame: Record<string, unknown> | null | undefined, at: number | undefined): boolean {
        if (!frame || frame.type !== "system") return false;
        const taskId = str(frame.task_id);
        if (!taskId) return false;
        switch (frame.subtype) {
            case "task_started": {
                const toolUseId = str(frame.tool_use_id);
                if (!toolUseId) return false;
                this.toolByTask.set(taskId, toolUseId);
                const pending = this.pendingByTask.get(taskId);
                if (!pending) return false;
                this.pendingByTask.delete(taskId);
                return this.record(toolUseId, pending);
            }
            case "task_notification": {
                const toolUseId = str(frame.tool_use_id) ?? this.toolByTask.get(taskId);
                return this.end(taskId, toolUseId, { status: terminalStatus(frame.status), endedAt: at });
            }
            case "task_updated": {
                const status = str((frame.patch as Record<string, unknown> | undefined)?.status);
                if (!status || !TERMINAL_UPDATE_STATUSES.has(status)) return false;
                return this.end(taskId, this.toolByTask.get(taskId), { status: terminalStatus(status), endedAt: at });
            }
            default:
                return false;
        }
    }

    private end(taskId: string, toolUseId: string | undefined, outcome: TaskOutcome): boolean {
        if (!toolUseId) {
            this.pendingByTask.set(taskId, outcome);
            return false;
        }
        return this.record(toolUseId, outcome);
    }

    private record(toolUseId: string, outcome: TaskOutcome): boolean {
        const prior = this.byTool.get(toolUseId);
        if (prior && prior.status === outcome.status && prior.endedAt === outcome.endedAt) return false;
        this.byTool.set(toolUseId, outcome);
        return true;
    }
}

/**
 * Close every `running` row the stream says ended. Pure; returns the same array
 * when nothing changes. A row with no usable end time gets its own start, so the
 * dock's retention window can still expire it instead of pinning it.
 */
export function applyStreamOutcomes(
    activities: readonly PinnedActivity[],
    outcomes: ReadonlyMap<string, TaskOutcome>,
): PinnedActivity[] {
    if (outcomes.size === 0) return activities as PinnedActivity[];
    let changed = false;
    const out = activities.map((a) => {
        const o = a.status === "running" ? outcomes.get(a.id) : undefined;
        if (!o) return a;
        changed = true;
        return { ...a, status: o.status, endedAt: o.endedAt ?? a.startedAt };
    });
    return changed ? out : (activities as PinnedActivity[]);
}

interface PaneOutcomes {
    tracker: TaskOutcomeTracker;
    snapshot: Accessor<ReadonlyMap<string, TaskOutcome>>;
    publish: (next: ReadonlyMap<string, TaskOutcome>) => void;
}

/**
 * One entry per REGISTERED pane. The lifetime is the pane-state slot's, not the
 * caller's: `agent-pane-registration.ts` opens an entry in `registerPane` and
 * drops it in `unregisterPane`, like the document and pane-state slots it sits
 * beside. A re-registered pane starts clean because its history is re-read.
 * Nothing here creates an entry on its own — a late frame for a closed pane
 * (an in-flight history page) is ignored instead of leaking one.
 */
const panes = new Map<string, PaneOutcomes>();

const EMPTY: ReadonlyMap<string, TaskOutcome> = new Map();
const NO_OUTCOMES: Accessor<ReadonlyMap<string, TaskOutcome>> = () => EMPTY;

/** Open (or reset) a pane's outcomes. Called by `registerPane`. */
export function registerTaskOutcomes(blockId: string): void {
    const tracker = new TaskOutcomeTracker();
    const [snapshot, setSnapshot] = createSignal<ReadonlyMap<string, TaskOutcome>>(EMPTY, { equals: false });
    panes.set(blockId, { tracker, snapshot, publish: (next) => setSnapshot(() => next) });
}

/** Drop a pane's outcomes. Called by `unregisterPane`; idempotent. */
export function unregisterTaskOutcomes(blockId: string): void {
    panes.delete(blockId);
}

/** The pane's stream-derived outcomes, reactive. Empty for a pane that isn't registered. */
export function taskOutcomesFor(blockId: string): Accessor<ReadonlyMap<string, TaskOutcome>> {
    return panes.get(blockId)?.snapshot ?? NO_OUTCOMES;
}

/** Feed a stdout frame for a pane (live or replayed). A new snapshot is
 *  published only when an outcome actually changed, so a burst of unrelated
 *  frames — a whole history page — causes no dock recompute. */
export function noteTaskFrame(blockId: string, frame: Record<string, unknown> | null | undefined, at: number | undefined): void {
    const p = panes.get(blockId);
    if (p?.tracker.note(frame, at)) p.publish(new Map(p.tracker.outcomes));
}
