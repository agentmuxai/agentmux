// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent's card says about its work (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §6.3): the repository
 * and branch its clone is on, from srv's work facts, and the Kanban tasks it holds with their pull requests. Both
 * are this instance's: an agent of another instance has neither. This file is pure; `atlas-work-store.ts` fetches.
 */

import { prLabel } from "../kanban/kanban-columns";
import type { KanbanTask, TaskState } from "../kanban/kanban-types";

/** One agent's entry in srv's work facts (`GET /api/v1/work-facts`): the fields the card reads. */
export interface WorkFact {
    block_id: string;
    repo?: string | null;
    branch?: string | null;
}

/** A task an agent holds, as its card lists it. */
export interface AtlasTask {
    num: number;
    title: string;
    /** Its pull request, and the short name the Kanban gives it (`#7292`); null when it has none yet. */
    pr: { url: string; label: string } | null;
}

export interface AtlasWork {
    repo: string | null;
    branch: string | null;
    tasks: AtlasTask[];
}

/** The most tasks a card lists: the ones last touched. */
export const MAX_CARD_TASKS = 3;

/** States in which a task is no longer anyone's work. */
const OVER: ReadonlySet<TaskState> = new Set<TaskState>(["done", "failed", "cancelled"]);

const text = (value: string | null | undefined): string | null => value?.trim() || null;

/**
 * The work of the agent in pane `blockId`: where its clone is, else where its newest task says the work is, and
 * the tasks it holds, newest first. Null when srv knows of neither.
 */
export function agentWork(blockId: string, facts: readonly WorkFact[], tasks: readonly KanbanTask[]): AtlasWork | null {
    const fact = facts.find((f) => f.block_id === blockId);
    const held = tasks
        .filter((t) => t.holder?.block_id === blockId && !OVER.has(t.state))
        .sort((a, b) => b.updated_at - a.updated_at);
    // The clone's own branch when srv has probed it; else the branch a held task was filed with.
    const filed = text(fact?.branch) ? undefined : held.find((t) => text(t.card.branch))?.card;
    const repo = filed ? text(filed.repo) : text(fact?.repo);
    const branch = filed ? text(filed.branch) : text(fact?.branch);
    if (!repo && !branch && held.length === 0) return null;
    return {
        repo,
        branch,
        tasks: held.slice(0, MAX_CARD_TASKS).map((t) => {
            const url = text(t.card.pr_url);
            return { num: t.card.num, title: t.title, pr: url ? { url, label: prLabel(url) } : null };
        }),
    };
}
