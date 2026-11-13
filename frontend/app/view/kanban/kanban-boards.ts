// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Several boards (SPEC_SHARED_KANBAN_ACROSS_MACHINES_2026_11_01.md §9): which board the pane shows, what
// each board's count of tasks needing the operator is, and how a task is named. srv lists every board's
// tasks in one answer (`kanban.list` with `board: "all"`); the pane shows the selected board's.

import { needsCountOf } from "./kanban-columns";
import type { KanbanBoard, KanbanBoardInfo, KanbanTask } from "./kanban-types";

/** The slug the default board has when srv names none (an older srv has one board, "main"). */
export const FALLBACK_BOARD = "main";

export function defaultSlugOf(raw: KanbanBoard | null): string {
    return raw?.default_board ?? raw?.boards?.find((b) => b.default)?.slug ?? FALLBACK_BOARD;
}

/** The slug of the board a task is on; a task srv gave no board is on the default one. */
export function boardSlugOf(t: KanbanTask, defaultSlug: string): string {
    return t.card.board || defaultSlug;
}

/** The board a request names, by slug: the one that exists, else the default board. A board that was
 *  deleted (or never existed) falls back instead of showing an empty board that can't be left. */
export function resolveBoard(boards: readonly KanbanBoardInfo[], wanted: string | null | undefined, defaultSlug: string): string {
    if (wanted && boards.some((b) => b.slug === wanted)) return wanted;
    return defaultSlug;
}

/** What the pane shows of `raw`: the selected board's tasks, with the same boards and agents. */
export function viewOfBoard(raw: KanbanBoard | null, selected: string | null | undefined): KanbanBoard | null {
    if (!raw) return null;
    const def = defaultSlugOf(raw);
    const slug = resolveBoard(raw.boards ?? [], selected, def);
    return { ...raw, board: slug, tasks: raw.tasks.filter((t) => boardSlugOf(t, def) === slug) };
}

/** The board's info for a slug. */
export function infoOf(boards: readonly KanbanBoardInfo[] | undefined, slug: string): KanbanBoardInfo | undefined {
    return boards?.find((b) => b.slug === slug);
}

/** How many tasks wait for the operator on each board, by slug. */
export function needsByBoard(raw: KanbanBoard | null, episodes: Parameters<typeof needsCountOf>[1] = []): Record<string, number> {
    const out: Record<string, number> = {};
    if (!raw) return out;
    const def = defaultSlugOf(raw);
    const bySlug = new Map<string, KanbanTask[]>();
    for (const t of raw.tasks) {
        const slug = boardSlugOf(t, def);
        bySlug.set(slug, [...(bySlug.get(slug) ?? []), t]);
    }
    for (const [slug, tasks] of bySlug) out[slug] = needsCountOf(tasks, episodes);
    return out;
}

/** The tasks of the boards that aren't archived: what the pane's title counts. */
export function tasksOnLiveBoards(raw: KanbanBoard | null): KanbanTask[] {
    if (!raw) return [];
    const def = defaultSlugOf(raw);
    const archived = new Set((raw.boards ?? []).filter((b) => b.archived).map((b) => b.slug));
    return raw.tasks.filter((t) => !archived.has(boardSlugOf(t, def)));
}

/** How a message names the task: `#12` on the default board, `work#12` elsewhere. */
export function refOf(t: KanbanTask): string {
    return t.card.task_ref || `#${t.card.num}`;
}

/** A task that is on a board other than the default one shows its full ref. */
export function isOffDefault(t: KanbanTask): boolean {
    return !!t.card.task_ref && !t.card.task_ref.startsWith("#");
}

/** The tier's words and icon for the picker. */
export function tierOf(tier: string): { icon: string; label: string } {
    if (tier === "cloud") return { icon: "cloud", label: "This machine and AgentMux Cloud" };
    if (tier === "lan") return { icon: "network-wired", label: "My machines" };
    return { icon: "desktop", label: "This machine only" };
}

/** Seconds left of a pending delete, never below zero. */
export function secondsLeft(pendingAt: number | null | undefined, now: number): number {
    return pendingAt ? Math.max(0, Math.ceil((pendingAt - now) / 1000)) : 0;
}
