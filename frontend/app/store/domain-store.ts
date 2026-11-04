// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One shared, live copy of a domain's data: a view reads it, and it is current without a remount.
 * docs/reports/REPORT_REACTIVE_COVERAGE_AUDIT_2026_11_03.md §5.
 *
 * A view that fetches once on mount shows stale data when something else changes it (an agent, another
 * window). The fix is not a signal per element: Solid already updates only what reads a changed field.
 * It is one subscription and one copy per domain, which this makes:
 *
 *   - `follow()` in a component keeps the store live while any component follows it, and stops it
 *     (events, poll) when the last one unmounts; the data stays for the next one.
 *   - srv events that mean "this changed" re-run `load`, a burst of them once (`debounceMs`).
 *   - an event that arrives while `load` runs makes it run once more after, so no change is missed
 *     and no two loads overlap.
 *   - after the websocket reconnects every followed store reloads: an event sent while it was down
 *     is gone, and nothing else replays it.
 *   - `safetyPollMs` reloads on a timer while followed, for a store whose events can be lost.
 *
 * It copies what the accounts cache (`identity-model.ts`), the Kanban (`kanban-held.tsx`) and the MCP
 * view (`mcp-observed.ts`) each did by hand.
 */

import { createSignal, onCleanup, type Accessor } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import type { MuxEventPayloads } from "@/app/store/mps-events.generated";
import { addWSReconnectHandler } from "@/app/store/ws";

export type DomainStoreOptions<T> = {
    /** What the store holds before the first load. */
    initial: T;
    /** Reads the whole domain from srv. */
    load: () => Promise<T>;
    /** The srv events that mean the domain changed. */
    events: ReadonlyArray<keyof MuxEventPayloads>;
    /** A burst of events reloads once, this long after the last (default 150). */
    debounceMs?: number;
    /** Also reload this often while followed. For a domain whose events can be lost. */
    safetyPollMs?: number;
    /** The value to keep from the old and new one, to hold the identity of what did not change. */
    merge?: (previous: T, next: T) => T;
};

export type DomainStore<T> = {
    data: Accessor<T>;
    /** False until a load has succeeded. */
    loaded: Accessor<boolean>;
    /** The last load's error, cleared by the next success. */
    error: Accessor<string | null>;
    /** Loads now (or once more after the load in flight). */
    refresh: () => Promise<void>;
    /** In a component: live while it is mounted. */
    follow: () => void;
};

const DEFAULT_DEBOUNCE_MS = 150;

/** The stores someone follows now, reloaded together after a reconnect. */
const followed = new Set<() => Promise<void>>();
let reconnectHooked = false;

function reloadAfterReconnect(): void {
    if (reconnectHooked) return;
    reconnectHooked = true;
    // `addWSReconnectHandler` has no unregister, so one handler serves every store.
    addWSReconnectHandler(() => {
        for (const refresh of followed) void refresh();
    });
}

export function createDomainStore<T>(options: DomainStoreOptions<T>): DomainStore<T> {
    const [data, setData] = createSignal<T>(options.initial);
    const [loaded, setLoaded] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    let loading = false;
    let again = false;
    let followers = 0;
    let unsubscribers: Array<() => void> = [];
    let debounce: ReturnType<typeof setTimeout> | null = null;
    let poll: ReturnType<typeof setInterval> | null = null;

    async function refresh(): Promise<void> {
        if (loading) {
            again = true;
            return;
        }
        loading = true;
        try {
            do {
                again = false;
                try {
                    const next = await options.load();
                    setData(() => (options.merge ? options.merge(data(), next) : next));
                    setError(null);
                    setLoaded(true);
                } catch (e) {
                    setError(e instanceof Error ? e.message : String(e));
                }
            } while (again);
        } finally {
            loading = false;
        }
    }

    function changed(): void {
        if (debounce) clearTimeout(debounce);
        debounce = setTimeout(() => {
            debounce = null;
            void refresh();
        }, options.debounceMs ?? DEFAULT_DEBOUNCE_MS);
    }

    function start(): void {
        void refresh();
        unsubscribers = options.events.map((event) => onMuxEvent(event, () => changed()));
        if (options.safetyPollMs) poll = setInterval(() => void refresh(), options.safetyPollMs);
        followed.add(refresh);
        reloadAfterReconnect();
    }

    function stop(): void {
        for (const unsubscribe of unsubscribers) unsubscribe();
        unsubscribers = [];
        if (debounce) clearTimeout(debounce);
        if (poll) clearInterval(poll);
        debounce = poll = null;
        followed.delete(refresh);
    }

    function follow(): void {
        if (followers++ === 0) start();
        onCleanup(() => {
            if (--followers === 0) stop();
        });
    }

    return { data, loaded, error, refresh, follow };
}
