// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent's card and node say about its work (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §6.3): the repository
 * and branch its clone is on, from srv's work facts, and its Kanban tasks (those it holds, then those waiting for it)
 * with their pull requests. All are this instance's: an agent of another instance has none. This file is pure;
 * `atlas-work-store.ts` fetches.
 */

import { backlogOrder, columnOf, prLabel, type ColumnId } from "../kanban/kanban-columns";
import type { KanbanTask, TaskState } from "../kanban/kanban-types";

/** One agent's entry in srv's work facts (`GET /api/v1/work-facts`): the fields the card reads. */
export interface WorkFact {
    block_id: string;
    repo?: string | null;
    branch?: string | null;
}

/** A task of an agent, as its node and its card list it. */
export interface AtlasTask {
    num: number;
    title: string;
    /** Its pull request, and the short name the Kanban gives it (`#7292`); null when it has none yet. */
    pr: { url: string; label: string } | null;
    /** The Kanban column it sits in: its colour (`--kanban-stage-<column>`) is the mark before it. */
    column: ColumnId;
    /** The agent holds it (working, asking, in review); false for one that waits for the agent to take it. */
    held: boolean;
}

/** Who an agent is to the board: a task names it by its uid or by its id (the slug), and a holder by its pane. */
export interface TaskOwner {
    blockId: string | null;
    agentId?: string | null;
    /** The name messages reach it by, when that is not its name. */
    route?: string | null;
    name: string;
}

export interface AtlasWork {
    repo: string | null;
    branch: string | null;
    tasks: AtlasTask[];
    /** Tasks beyond the ones listed. */
    more: number;
}

/** The most tasks a card lists: the held ones first, then those waiting, the rest said as a count. */
export const MAX_CARD_TASKS = 6;
/** The most tasks a node on the map lists; with more, its last line says how many are left ("+2 more"). */
export const MAX_NODE_TASKS = 3;

/** States in which a task is no longer anyone's work. */
const OVER: ReadonlySet<TaskState> = new Set<TaskState>(["done", "failed", "cancelled"]);

const text = (value: string | null | undefined): string | null => value?.trim() || null;

const atlasTask = (t: KanbanTask, held: boolean): AtlasTask => {
    const url = text(t.card.pr_url);
    return {
        num: t.card.num,
        title: t.title,
        pr: url ? { url, label: prLabel(url) } : null,
        column: columnOf(t),
        held,
    };
};

/** A task not yet taken that is for `owner`: by its uid, or by its id or name in any case. */
function waitsFor(t: KanbanTask, owner: TaskOwner): boolean {
    if (t.holder || (t.state !== "open" && t.state !== "held")) return false;
    if (owner.agentId && t.target_agent_uid && t.target_agent_uid === owner.agentId) return true;
    const names = [owner.route, owner.name].flatMap((n) => (n ? [n.toLowerCase()] : []));
    return !!t.target_agent && names.includes(t.target_agent.toLowerCase());
}

/**
 * The tasks of `owner`, in the order a node lists them: those it holds first (newest first), then those waiting for it
 * in the order srv hands them out. A task that is over is no one's. Without a pane, nothing is held.
 */
export function tasksOf(owner: TaskOwner, tasks: readonly KanbanTask[]): AtlasTask[] {
    const held = owner.blockId
        ? tasks
              .filter((t) => t.holder?.block_id === owner.blockId && !OVER.has(t.state))
              .sort((a, b) => b.updated_at - a.updated_at)
        : [];
    const waiting = tasks.filter((t) => waitsFor(t, owner)).sort(backlogOrder);
    return [...held.map((t) => atlasTask(t, true)), ...waiting.map((t) => atlasTask(t, false))];
}

/** What a node lists of `all`: the first `cap`, and how many more there are. */
export interface NodeTasks {
    shown: AtlasTask[];
    more: number;
    total: number;
}

export function nodeTasks(all: readonly AtlasTask[], cap = MAX_NODE_TASKS): NodeTasks {
    return { shown: all.slice(0, cap), more: Math.max(0, all.length - cap), total: all.length };
}

/**
 * The work of the agent in pane `blockId`: where its clone is, else where its newest held task says the work is, and
 * its tasks. With `owner` the tasks waiting for the agent follow those it holds. Null when srv knows of neither.
 */
export function agentWork(
    blockId: string,
    facts: readonly WorkFact[],
    tasks: readonly KanbanTask[],
    owner?: Omit<TaskOwner, "blockId">
): AtlasWork | null {
    const fact = facts.find((f) => f.block_id === blockId);
    const mine = tasksOf({ name: "", ...owner, blockId }, tasks);
    const held = tasks
        .filter((t) => t.holder?.block_id === blockId && !OVER.has(t.state))
        .sort((a, b) => b.updated_at - a.updated_at);
    // The clone's own branch when srv has probed it; else the branch a held task was filed with.
    const filed = text(fact?.branch) ? undefined : held.find((t) => text(t.card.branch))?.card;
    const repo = filed ? text(filed.repo) : text(fact?.repo);
    const branch = filed ? text(filed.branch) : text(fact?.branch);
    if (!repo && !branch && mine.length === 0) return null;
    return { repo, branch, tasks: mine.slice(0, MAX_CARD_TASKS), more: Math.max(0, mine.length - MAX_CARD_TASKS) };
}
