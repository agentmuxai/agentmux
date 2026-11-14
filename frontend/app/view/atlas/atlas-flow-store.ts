// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The lines on show on one Atlas pane's map (atlas-flow.ts), kept up to date: srv's `message-flow` and `task-flow`
 * events add one as they happen, the recent lists (`GET /api/v1/message-flow` and `/task-flow`) fill in what
 * happened just before the pane opened, and a clock that runs only while there is a line takes each away when its
 * time is up. A pull request's review and checks (atlas-pr.ts) are lines too: `review-flow` and `checks-flow` add
 * or replace one, `merge-flow` ends the pull request's, and `GET /api/v1/pr-flow` gives the recent ones.
 */

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { muxbusDelivery, RECONNECTING_QUIET_MS } from "@/app/store/muxbus-delivery";
import type { MessageFlowEvent } from "@/types/rpc/MessageFlowEvent";
import type { PrFlowSnapshot } from "@/types/rpc/PrFlowSnapshot";
import type { PrRef } from "@/types/rpc/PrRef";
import type { TaskFlowEvent } from "@/types/rpc/TaskFlowEvent";
import { createEffect, createSignal, onCleanup, type Accessor } from "solid-js";
import { flowsAt, messageFlow, taskFlow, withFlow, withoutPr, type AtlasFlow } from "./atlas-flow";
import { checksFlow, prKey, reviewFlow } from "./atlas-pr";
import { srvJson, srvList } from "./atlas-srv";

const TICK_MS = 500;

export interface AtlasFlows {
    /** The lines to draw now. */
    flows: Accessor<readonly AtlasFlow[]>;
    /** When the newest review, checks or merge notice came, as srv or this window saw it; null for none yet. */
    lastPrNoticeAtMs: Accessor<number | null>;
}

const listOf = <T>(list: T[] | undefined): T[] => (Array.isArray(list) ? list : []);

/** The lines to draw now, following srv for as long as the caller lives. */
export function useAtlasFlows(): AtlasFlows {
    const [flows, setFlows] = createSignal<readonly AtlasFlow[]>([]);
    const [lastPrNoticeAtMs, setLastPrNotice] = createSignal<number | null>(null);
    /** The pull requests that have ended: a notice about one that comes late draws nothing. */
    const ended = new Set<string>();
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
        if (!flow || (flow.pr && ended.has(flow.pr))) return;
        const now = Date.now();
        // Timed again once it is in: a line from the recent list may be past its first minutes already.
        const next = setFlows((list) => flowsAt(withFlow(flowsAt(list, now), flow, now), now));
        if (next.length > 0 && clock === undefined) clock = setInterval(tick, TICK_MS);
    };
    // A line that happens now is timed from now, by this window's clock; one from the recent list from when it happened.
    const live = (flow: AtlasFlow | null) => add(flow && { ...flow, atMs: Date.now() });
    const noticed = (atMs: number | null | undefined) => {
        if (typeof atMs === "number") setLastPrNotice((last) => Math.max(last ?? 0, atMs));
    };
    const end = (pr: PrRef | undefined) => {
        const key = prKey(pr);
        if (!key) return;
        ended.add(key);
        setFlows((list) => withoutPr(list, key));
    };

    let disposed = false;
    const unsubscribe = [
        onMuxEvent(WpsEvent.MessageFlow, (event) => live(messageFlow(event))),
        onMuxEvent(WpsEvent.TaskFlow, (event) => live(taskFlow(event))),
        onMuxEvent(WpsEvent.ReviewFlow, (event) => {
            noticed(event.at_ms);
            live(reviewFlow(event));
        }),
        onMuxEvent(WpsEvent.ChecksFlow, (event) => {
            noticed(event.at_ms);
            live(checksFlow(event));
        }),
        onMuxEvent(WpsEvent.MergeFlow, (event) => {
            noticed(event.at_ms);
            end(event.pr);
        }),
    ];
    void Promise.all([
        srvList<MessageFlowEvent>("/api/v1/message-flow", "messages"),
        srvList<TaskFlowEvent>("/api/v1/task-flow", "events"),
        srvJson<PrFlowSnapshot>("/api/v1/pr-flow"),
    ]).then(([messages, tasks, pr]) => {
        if (disposed) return;
        for (const event of messages) add(messageFlow(event));
        for (const event of tasks) add(taskFlow(event));
        noticed(pr?.bot?.last_notice_at_ms);
        // The ends first: a pull request that has merged draws nothing.
        for (const event of listOf(pr?.merge)) end(event.pr);
        for (const event of listOf(pr?.review)) add(reviewFlow(event));
        for (const event of listOf(pr?.checks)) add(checksFlow(event));
    });
    onCleanup(() => {
        disposed = true;
        stop();
        for (const off of unsubscribe) off();
    });
    return { flows, lastPrNoticeAtMs };
}

/**
 * Whether GitHub's notices are not reaching this machine: MuxBus needs a sign-in, or has been out of reach for
 * longer than the status bar stays quiet about (a reconnect that recovers sooner is not worth saying).
 */
export function useCloudPaused(): Accessor<boolean> {
    const [late, setLate] = createSignal(false);
    createEffect(() => {
        const status = muxbusDelivery();
        setLate(false);
        if (status?.state !== "reconnecting") return;
        const timer = setTimeout(() => setLate(true), Math.max(0, status.since_ms + RECONNECTING_QUIET_MS - Date.now()));
        onCleanup(() => clearTimeout(timer));
    });
    return () => muxbusDelivery()?.state === "needs_sign_in" || late();
}
