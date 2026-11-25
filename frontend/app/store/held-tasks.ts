// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Kanban task each agent pane holds, by the pane's block id, as the window last read the boards: what an agent tab's
 * hover card says of its agent's work. The Kanban publishes it with the tasks it already fetches (kanban-links.ts), and
 * a window with no Kanban open follows the queue itself (kanban-task-feed.ts), so reading it costs no request: while the
 * boards have not loaded the map is empty and the card simply has no task row.
 */

import { createSignal } from "solid-js";

export interface HeldTask {
    num: number;
    title: string;
}

/** What of a Kanban task this needs (a `KanbanTask` has it): who holds it, its state and its number and title. */
export interface HeldTaskSource {
    title: string;
    state: string;
    updated_at: number;
    holder: { block_id?: string | null } | null;
    card: { num: number };
}

const HELD: ReadonlySet<string> = new Set(["claimed", "blocked", "review"]);

const [held, setHeld] = createSignal<ReadonlyMap<string, HeldTask>>(new Map());
let signature = "";

/** The tasks as the boards have them now: the newest claimed, asking or in-review task of each pane. */
export function publishHeldTasks(tasks: readonly HeldTaskSource[]): void {
    const byPane = new Map<string, HeldTaskSource>();
    for (const t of tasks) {
        const pane = t.holder?.block_id;
        if (!pane || !HELD.has(t.state)) continue;
        const seen = byPane.get(pane);
        if (!seen || t.updated_at > seen.updated_at) byPane.set(pane, t);
    }
    const next = [...byPane].map(([pane, t]) => `${pane}:${t.card.num}:${t.title}`).sort().join("|");
    if (next === signature) return;
    signature = next;
    setHeld(new Map([...byPane].map(([pane, t]) => [pane, { num: t.card.num, title: t.title }])));
}

/** Reactive: the task the pane in block `blockId` holds, if the boards say so. */
export function heldTaskOf(blockId: string): HeldTask | undefined {
    return held().get(blockId);
}

let follower: (() => void) | undefined;

/** The Kanban's feed says how a window follows the queue (app-init.ts), so a surface below the views can ask for it. */
export function registerTaskFollower(follow: () => void): void {
    follower = follow;
}

/** Idempotent: has the window read the boards, so a held task has its place in the map. */
export function followHeldTasks(): void {
    follower?.();
}
