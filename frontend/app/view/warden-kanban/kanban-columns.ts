// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Where each task sits on the Kanban, and in what order (docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md
// §6.3, §6.5). Pure: the section and its tests share it.

import type { KanbanAfter, KanbanAgent, KanbanTask } from "./kanban-types";

export type ColumnId = "backlog" | "progress" | "needs" | "review" | "done";

export const COLUMNS: { id: ColumnId; label: string }[] = [
    { id: "backlog", label: "Backlog" },
    { id: "progress", label: "In progress" },
    { id: "needs", label: "Needs you" },
    { id: "review", label: "Review" },
    { id: "done", label: "Done" },
];

/** Why a task needs the operator, or null when it doesn't. The first line of its card in Needs you. */
export function needsReason(t: KanbanTask): string | null {
    switch (t.state) {
        case "blocked":
            return t.card.question ? `asks: ${t.card.question}` : "asks you something";
        case "failed":
            return `failed after ${t.attempts} attempt${t.attempts === 1 ? "" : "s"}`;
        case "claimed": {
            const h = t.holder;
            if (!h || h.state === "gone") return "agent closed";
            if (h.state === "waiting") return "question waiting in its pane";
            if (h.state === "error") return "agent errored";
            if (h.state === "stopped") return "agent stopped";
            return null;
        }
        default:
            return null;
    }
}

/** The column a task belongs in: its stored state and its holder's live state. */
export function columnOf(t: KanbanTask): ColumnId {
    switch (t.state) {
        case "open":
        case "held":
            return "backlog";
        case "review":
            return "review";
        case "done":
        case "cancelled":
            return "done";
        case "blocked":
        case "failed":
            return "needs";
        case "claimed":
            return needsReason(t) ? "needs" : "progress";
        default:
            return "backlog";
    }
}

/** A waiting task's place in its backlog: the operator's rank, else when it was filed (srv's claim order). */
export function rankOf(t: KanbanTask): number {
    return t.card.rank ?? t.created_at;
}

/** srv's claim order within one agent's backlog: priority, then rank. */
export function backlogOrder(a: KanbanTask, b: KanbanTask): number {
    return b.priority - a.priority || rankOf(a) - rankOf(b);
}

/** A rank that puts a task between two others (either end may be missing). */
export function rankBetween(before: number | undefined, after: number | undefined): number {
    if (before === undefined && after === undefined) return Date.now();
    if (before === undefined) return (after as number) - 1000;
    if (after === undefined) return before + 1000;
    return (before + after) / 2;
}

export interface BacklogGroup {
    /** The agent's name, or "" for Anyone. */
    key: string;
    label: string;
    agent: KanbanAgent | undefined;
    tasks: KanbanTask[];
}

/** The Backlog grouped by who each task waits for: one group per agent, alphabetically, then Anyone. */
export function backlogGroups(tasks: KanbanTask[], agents: KanbanAgent[]): BacklogGroup[] {
    const groups = new Map<string, KanbanTask[]>();
    for (const t of tasks) {
        if (columnOf(t) !== "backlog") continue;
        const key = t.target_agent || "";
        groups.set(key, [...(groups.get(key) ?? []), t]);
    }
    const out: BacklogGroup[] = [...groups.entries()].map(([key, list]) => ({
        key,
        label: key || "Anyone",
        agent: key ? agents.find((a) => a.name === key || (!!a.uid && a.uid === list[0]?.target_agent_uid)) : undefined,
        tasks: [...list].sort(backlogOrder),
    }));
    return out.sort((a, b) => (a.key === "" ? 1 : b.key === "" ? -1 : a.label.localeCompare(b.label)));
}

/** Tasks of one column, in the order the column shows them. */
export function columnTasks(tasks: KanbanTask[], column: ColumnId): KanbanTask[] {
    const list = tasks.filter((t) => columnOf(t) === column);
    if (column === "backlog") return list.sort(backlogOrder);
    // Newest change first everywhere else: what just moved is on top.
    return list.sort((a, b) => b.updated_at - a.updated_at);
}

/** Agents at work with no task on the board: shown as untracked cards (§6.6). */
export function untrackedAgents(agents: KanbanAgent[], tasks: KanbanTask[]): KanbanAgent[] {
    const holding = new Set(
        tasks
            .filter((t) => t.state === "claimed" || t.state === "blocked")
            .flatMap((t) => [t.claimed_by_uid, t.claimed_by].filter((x) => x))
    );
    return agents.filter(
        (a) => (a.state === "working" || a.state === "waiting") && !holding.has(a.uid ?? "") && !holding.has(a.name)
    );
}

/** What the Review column header says: how many, and how long the oldest has waited. */
export function reviewLoad(tasks: KanbanTask[], now: number): { count: number; oldestMs: number } {
    const inReview = tasks.filter((t) => t.state === "review");
    const oldest = inReview.reduce((min, t) => Math.min(min, t.updated_at), now);
    return { count: inReview.length, oldestMs: inReview.length ? now - oldest : 0 };
}

/** Who asked for a task, in words. */
export function requestedBy(t: KanbanTask): string {
    const c = t.card;
    if (c.requested_by_kind === "operator") return c.filed_by ? `you, via ${c.filed_by}` : "you";
    if (c.requested_by_kind === "agent" && c.requested_by) return c.requested_by;
    return c.filed_by || t.created_by || "unknown";
}

/** The checklist's progress, "3/7", or null without one. */
export function checklistProgress(t: KanbanTask): string | null {
    const list = t.card.checklist ?? [];
    if (!list.length) return null;
    return `${list.filter((i) => i.status === "completed").length}/${list.length}`;
}

/** The tasks this one starts after that aren't finished yet: while any is, nobody takes it. */
export function waitingOn(t: KanbanTask): KanbanAfter[] {
    return (t.card.after ?? []).filter((a) => a.state !== "done" && a.state !== "cancelled");
}

/** "#12, 15" → ["12", "15"]: the numbers typed for "starts after". */
export function parseAfter(text: string): string[] {
    return text
        .split(/[\s,;]+/)
        .map((s) => s.replace(/^#/, ""))
        .filter(Boolean);
}

/** Whether a task matches the search box: title, number, agent, repository. */
export function matchesQuery(t: KanbanTask, q: string): boolean {
    const s = q.trim().toLowerCase();
    if (!s) return true;
    if (s.replace(/^#/, "") === String(t.card.num)) return true;
    return [t.title, t.target_agent, t.claimed_by, t.card.repo, t.card.branch, t.kind]
        .filter(Boolean)
        .some((f) => f.toLowerCase().includes(s));
}
