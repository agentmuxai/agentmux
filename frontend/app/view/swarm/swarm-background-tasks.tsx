// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Background-task rows in the Swarm: the agent-level "Background" bucket, and
 * the list nested under a subagent row for the tasks that subagent launched.
 * Data and grouping come from swarm-background.ts; this file only renders.
 * SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md §4.
 */

import { createMemo, For, Show, type JSX } from "solid-js";
import type { BackgroundTaskView } from "@/app/store/rpc-api";
import { showCopyContextMenu } from "@/app/store/contextmenu";
import { useTick } from "@/app/hook/useTick";
import { formatElapsedClock } from "@/util/format-time";
import { isBackgroundTaskVisible } from "./swarm-background";
import "./swarm-background-tasks.scss";

const STATUS_TEXT: Record<BackgroundTaskView["status"], string> = {
    running: "running",
    done: "done",
    error: "failed",
    stopped: "stopped",
};

/** Pure: the tasks still shown at `now`, in the order given. */
export function visibleBackgroundTasks(tasks: readonly BackgroundTaskView[], now: number): BackgroundTaskView[] {
    return tasks.filter((t) => isBackgroundTaskVisible(t, now));
}

/** The agent's own background tasks (and any whose subagent has no row). */
export function BackgroundTaskBucket(props: { tasks: BackgroundTaskView[] }): JSX.Element {
    return (
        <Show when={props.tasks.length > 0}>
            <div class="swarm-bucket swarm-bucket--background">
                <div class="swarm-bucket-header">
                    <span class="swarm-bucket-label">Background</span>
                    <span class="swarm-bucket-count">{props.tasks.length}</span>
                </div>
                <For each={props.tasks}>{(task) => <BackgroundTaskRow task={task} />}</For>
            </div>
        </Show>
    );
}

/** The background tasks one subagent launched, under its row. */
export function SubagentBackgroundTasks(props: { tasks: BackgroundTaskView[] }): JSX.Element {
    return (
        <Show when={props.tasks.length > 0}>
            <div class="swarm-subagent-background">
                <For each={props.tasks}>{(task) => <BackgroundTaskRow task={task} nested />}</For>
            </div>
        </Show>
    );
}

function BackgroundTaskRow(props: { task: BackgroundTaskView; nested?: boolean }): JSX.Element {
    const tick = useTick(1000);
    // A finished task shows how long it ran, frozen; a running one keeps counting.
    const elapsed = createMemo(() => {
        const t = props.task;
        if (t.status !== "running") {
            return formatElapsedClock(Math.max(0, (t.ended_at_ms ?? t.last_seen_ms) - t.started_at_ms));
        }
        tick();
        return formatElapsedClock(Math.max(0, Date.now() - t.started_at_ms));
    });
    return (
        <div
            classList={{
                "swarm-background-row": true,
                "swarm-background-row--nested": !!props.nested,
                [`swarm-background-row--${props.task.status}`]: true,
            }}
            title={props.task.label}
            onContextMenu={(e) => showCopyContextMenu([{ label: "Copy description", value: props.task.label }], e)}
        >
            <span class="swarm-background-sigil">$</span>
            <span class="swarm-background-title">{props.task.label}</span>
            <span class="swarm-background-status">{STATUS_TEXT[props.task.status]}</span>
            <span class="swarm-background-elapsed">{elapsed()}</span>
        </div>
    );
}
