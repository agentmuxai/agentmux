// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The task index for a window with no Kanban open (links spec §4.3): the numbers only, loaded once when a surface
 * first needs them, and again when the queue changes. A Kanban that is open publishes the same data itself.
 */

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { publishBoardTasks } from "./kanban-links";

const DEBOUNCE_MS = 500;

let following = false;
let timer: ReturnType<typeof setTimeout> | null = null;

async function load(): Promise<void> {
    try {
        publishBoardTasks(await RpcApi.KanbanListCommand(TabRpcClient, { board: "all" }));
    } catch (err) {
        // No task index: a number stays plain text until the next change of the queue.
        console.warn("[links] couldn't load the tasks", err);
    }
}

/** Idempotent: starts the first load and the follow of the queue. */
export function followTasks(): void {
    if (following) return;
    following = true;
    void load();
    onMuxEvent(WpsEvent.WorkQueueChanged, () => {
        if (timer) clearTimeout(timer);
        timer = setTimeout(() => {
            timer = null;
            void load();
        }, DEBOUNCE_MS);
    });
}
