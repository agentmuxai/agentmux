// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tasks the viewer can see, held once for the window (spec §4.3). The Kanban publishes what it already fetches
 * after each refresh, so linking a task number costs no request of its own; a surface that is shown with no Kanban
 * open gets its own load in the build that adds it.
 */

import { createSignal } from "solid-js";
import { buildTaskIndex } from "./entities/indexes";
import type { TaskEntry, TaskIndex } from "./entities/types";

let version = 0;
let signature = "";
const [index, setIndex] = createSignal<TaskIndex>(buildTaskIndex([], "main", 0));

/** The tasks as the Kanban has them now. An unchanged set is not published again, so linked text is not recomputed. */
export function publishTasks(tasks: TaskEntry[], defaultBoard: string): void {
    const next = `${defaultBoard}|${tasks.map((t) => `${t.board}#${t.number}`).join(",")}`;
    if (next === signature) return;
    signature = next;
    setIndex(buildTaskIndex(tasks, defaultBoard, ++version));
}

/** Reactive: read inside a computation, it follows the published tasks. */
export function taskIndex(): TaskIndex {
    return index();
}
