// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban section's state: the board from srv (`kanban.list`), refreshed when the task queue
// changes (`workqueue:changed`) and every 15 s while the section is visible, because a task an agent
// of another AgentMux on this computer moves raises no event here (SPEC_WARDEN_KANBAN_2026_10_20.md
// §7.3). Nothing is fetched or subscribed while the section is hidden: Warden keeps every section
// mounted.

import { createMemo, createSignal, type Accessor } from "solid-js";

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

import { columnOf } from "./kanban-columns";
import type { KanbanActRequest, KanbanActResult, KanbanBoard, KanbanTaskDetail } from "./kanban-types";

const POLL_MS = 15_000;
const DEBOUNCE_MS = 150;

export class KanbanModel {
    readonly board: Accessor<KanbanBoard | null>;
    readonly error: Accessor<string | null>;
    /** How many tasks need the operator: the Warden tab's count. */
    readonly needsCount: Accessor<number>;
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
        this.needsCount = createMemo(() => (board()?.tasks ?? []).filter((t) => columnOf(t) === "needs").length);
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
            this.setBoard(board);
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
