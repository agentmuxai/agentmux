// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The lines on show on one Atlas pane's map (atlas-flow.ts), kept up to date: srv's `message-flow` and `task-flow`
 * events add one as they happen, the recent lists (`GET /api/v1/message-flow` and `/task-flow`) fill in what
 * happened just before the pane opened, and a clock that runs only while there is a line takes each away when its
 * time is up.
 */

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import type { MessageFlowEvent } from "@/types/rpc/MessageFlowEvent";
import type { TaskFlowEvent } from "@/types/rpc/TaskFlowEvent";
import { createSignal, onCleanup, type Accessor } from "solid-js";
import { flowsAt, messageFlow, taskFlow, withFlow, type AtlasFlow } from "./atlas-flow";
import { srvList } from "./atlas-srv";

const TICK_MS = 500;

/** The lines to draw now, following srv for as long as the caller lives. */
export function useAtlasFlows(): Accessor<readonly AtlasFlow[]> {
    const [flows, setFlows] = createSignal<readonly AtlasFlow[]>([]);
    let clock: ReturnType<typeof setInterval> | undefined;
    const stop = () => {
        clearInterval(clock);
        clock = undefined;
    };
    const tick = () => {
        const next = setFlows((list) => flowsAt(list, Date.now()));
        if (next.length === 0) stop();
    };
    const add = (flow: AtlasFlow | null) => {
        if (!flow) return;
        const now = Date.now();
        const next = setFlows((list) => withFlow(flowsAt(list, now), flow, now));
        if (next.length > 0 && clock === undefined) clock = setInterval(tick, TICK_MS);
    };
    // A line that happens now is timed from now, by this window's clock; one from the recent list from when it happened.
    const live = (flow: AtlasFlow | null) => add(flow && { ...flow, atMs: Date.now() });

    let disposed = false;
    const unsubscribe = [
        onMuxEvent(WpsEvent.MessageFlow, (event) => live(messageFlow(event))),
        onMuxEvent(WpsEvent.TaskFlow, (event) => live(taskFlow(event))),
    ];
    void Promise.all([
        srvList<MessageFlowEvent>("/api/v1/message-flow", "messages"),
        srvList<TaskFlowEvent>("/api/v1/task-flow", "events"),
    ]).then(([messages, tasks]) => {
        if (disposed) return;
        for (const event of messages) add(messageFlow(event));
        for (const event of tasks) add(taskFlow(event));
    });
    onCleanup(() => {
        disposed = true;
        stop();
        for (const off of unsubscribe) off();
    });
    return flows;
}
