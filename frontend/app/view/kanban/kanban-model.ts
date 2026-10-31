// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban section's state: the board from srv (`kanban.list`), refreshed when the task queue
// changes (`workqueue:changed`) and every 15 s while the section is visible, because a task an agent
// of another AgentMux on this computer moves raises no event here (SPEC_WARDEN_KANBAN_2026_10_20.md
// §7.3). Nothing is fetched or subscribed while the section is hidden: the Swarm keeps the section
// mounted.

import { createSignal, type Accessor } from "solid-js";

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

import type { KanbanActRequest, KanbanActResult, KanbanBoard, KanbanTask, KanbanTaskDetail } from "./kanban-types";

const POLL_MS = 15_000;
const DEBOUNCE_MS = 150;

/** The task a request to open one names: its id, or its number as the operator sees it ("12", from the
 *  link agentmux://task/12). Exported for its test. */
export function taskForOpenRequest(tasks: readonly KanbanTask[], request: string): KanbanTask | undefined {
    return tasks.find((t) => t.id === request || String(t.card.num) === request);
}

/** The new board, keeping each task and agent object that didn't change, so the cards showing them stay
 *  mounted across a refresh: a focused card keeps focus, a drag keeps its source, an open drawer doesn't
 *  refetch. Exported for its test. */
export function keepUnchanged(prev: KanbanBoard | null, next: KanbanBoard): KanbanBoard {
    if (!prev) return next;
    const same = <T,>(old: T[], fresh: T[], key: (x: T) => string): T[] => {
        const byKey = new Map(old.map((x) => [key(x), x]));
        return fresh.map((x) => {
            const o = byKey.get(key(x));
            return o !== undefined && JSON.stringify(o) === JSON.stringify(x) ? o : x;
        });
    };
    return {
        ...next,
        tasks: same(prev.tasks, next.tasks, (t) => t.id),
        agents: same(prev.agents, next.agents, (a) => a.block_id),
    };
}

export class KanbanModel {
    readonly board: Accessor<KanbanBoard | null>;
    readonly error: Accessor<string | null>;
    private readonly setBoard: (b: KanbanBoard | null) => void;
    private readonly setError: (e: string | null) => void;
    private unsub: (() => void) | null = null;
    private poll: ReturnType<typeof setInterval> | null = null;
    private debounce: ReturnType<typeof setTimeout> | null = null;
    private inflight = false;
    private again = false;

    constructor() {
        const [board, setBoard] = createSignal<KanbanBoard | null>(null);
        const [error, setError] = createSignal<string | null>(null);
        this.board = board;
        this.error = error;
        this.setBoard = setBoard;
        this.setError = setError;
    }

    /** Follow the queue while the section is shown; stop when it isn't. */
    setVisible(visible: boolean): void {
        if (visible && !this.unsub) {
            this.unsub = onMuxEvent(WpsEvent.WorkQueueChanged, () => this.scheduleRefresh());
            this.poll = setInterval(() => void this.refresh(), POLL_MS);
            void this.refresh();
        } else if (!visible && this.unsub) {
            this.stop();
        }
    }

    private stop(): void {
        this.unsub?.();
        this.unsub = null;
        if (this.poll) clearInterval(this.poll);
        this.poll = null;
        if (this.debounce) clearTimeout(this.debounce);
        this.debounce = null;
    }

    private scheduleRefresh(): void {
        if (this.debounce) clearTimeout(this.debounce);
        this.debounce = setTimeout(() => {
            this.debounce = null;
            void this.refresh();
        }, DEBOUNCE_MS);
    }

    /** Fetch the board; a call while one is running runs once more after it. */
    async refresh(): Promise<void> {
        if (this.inflight) {
            this.again = true;
            return;
        }
        this.inflight = true;
        try {
            const board = await RpcApi.KanbanListCommand(TabRpcClient, {});
            this.setBoard(keepUnchanged(this.board(), board));
            this.setError(null);
        } catch (e) {
            this.setError(`Couldn't load the Kanban: ${String((e as Error)?.message ?? e)}`);
        } finally {
            this.inflight = false;
            if (this.again) {
                this.again = false;
                void this.refresh();
            }
        }
    }

    /** One of the operator's moves. Returns srv's answer, or null after showing why it failed. */
    async act(req: KanbanActRequest): Promise<KanbanActResult | null> {
        try {
            const res = await RpcApi.KanbanActCommand(TabRpcClient, req);
            this.setError(null);
            void this.refresh();
            return res;
        } catch (e) {
            this.setError(String((e as Error)?.message ?? e));
            return null;
        }
    }

    /** One task with its history, for the drawer. */
    async detail(id: string): Promise<KanbanTaskDetail | null> {
        try {
            return await RpcApi.KanbanTaskCommand(TabRpcClient, { id });
        } catch (e) {
            this.setError(String((e as Error)?.message ?? e));
            return null;
        }
    }

    dismissError(): void {
        this.setError(null);
    }

    dispose(): void {
        this.stop();
    }
}
