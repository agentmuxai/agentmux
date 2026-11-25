// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tasks on the map's nodes (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §6.3): which tasks each of this instance's
 * agents lists, how tall that makes its node, and whether the map is zoomed far enough in to list them at all.
 * Pure: `atlas-work-store.ts` brings the Kanban's tasks.
 */

import type { KanbanTask } from "../kanban/kanban-types";
import type { AtlasAgent } from "./atlas-graph";
import { nodeHeight, type NodeHeights } from "./atlas-layout";
import { nodeTasks, tasksOf, type NodeTasks } from "./atlas-work";

/**
 * The scale from which a node lists its tasks line by line: its 10 px lines are then 8 px, the smallest where a task's
 * number and the start of its title still read. Below it a node says only how many tasks it has.
 */
export const TASK_LINES_MIN_K = 0.8;

/** What each of `agents` that has tasks lists, by node key. An agent of another instance has none here: its board is its machine's. */
export function nodeTasksOf(agents: readonly AtlasAgent[], tasks: readonly KanbanTask[]): Map<string, NodeTasks> {
    const out = new Map<string, NodeTasks>();
    if (tasks.length === 0) return out;
    for (const agent of agents) {
        if (agent.remote) continue;
        const mine = tasksOf(agent, tasks);
        if (mine.length > 0) out.set(agent.key, nodeTasks(mine));
    }
    return out;
}

/** The lines a node lists: its tasks and, past the cap, one that says how many more. */
export const linesOf = (t: NodeTasks): number => t.shown.length + (t.more > 0 ? 1 : 0);

/** How tall each node that lists tasks is, by node key. */
export function nodeHeights(tasks: ReadonlyMap<string, NodeTasks>): NodeHeights {
    return new Map([...tasks].map(([key, t]) => [key, nodeHeight(linesOf(t))]));
}

/**
 * Whether nodes list their tasks: when the fitted view of the map with the lines (`fitK`, its scale) reads them, or when
 * the person has zoomed in (`touched`) to a scale (`k`) that does. A busy map fits small and shows counts; zooming in shows the lines.
 */
export function showTaskLines(fitK: number, touched: boolean, k: number): boolean {
    return fitK >= TASK_LINES_MIN_K || (touched && k >= TASK_LINES_MIN_K);
}
