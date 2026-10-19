// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Updates to the Swarm's per-block maps that do nothing when nothing changed.
 *
 * The status and progress events arrive often (every controller status event, every todo or
 * tool change of every agent), and each handler replaced the whole Map with a copy. A copy is a
 * new signal value, so `buildTree` re-ran and the rows were recomputed for an update that
 * changed nothing: ~5% of the page with 12 agents streaming (lab CPU profile). Returning the
 * same Map when the entry is unchanged leaves the signal alone.
 */

import type { AgentProgress } from "./swarm-model";

/** `prev` with `blockId` set to `status`; `prev` itself when it already is. */
export function withAgentStatus<S extends string>(prev: Map<string, S>, blockId: string, status: S): Map<string, S> {
    return prev.get(blockId) === status ? prev : new Map(prev).set(blockId, status);
}

/** The same todo list, tool and flags: what the Swarm shows of a block's progress. */
export function sameProgress(a: AgentProgress | undefined, b: AgentProgress): boolean {
    if (!a) return false;
    if (a.currentTool !== b.currentTool || a.todosTruncated !== b.todosTruncated || a.todosPartial !== b.todosPartial) return false;
    return a.todos.length === b.todos.length && JSON.stringify(a.todos) === JSON.stringify(b.todos);
}

/** `prev` with `blockId`'s progress set to `next`; `prev` itself when it already shows the same. */
export function withAgentProgress(
    prev: Map<string, AgentProgress>,
    blockId: string,
    next: AgentProgress
): Map<string, AgentProgress> {
    return sameProgress(prev.get(blockId), next) ? prev : new Map(prev).set(blockId, next);
}
