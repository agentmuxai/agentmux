// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const handlers = new Map<string, Set<() => void>>();
const unsubscribed: string[] = [];
let reconnect: (() => void) | null = null;
let reconnectHandlers = 0;
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (event: string, handler: () => void) => {
        if (!handlers.has(event)) handlers.set(event, new Set());
        handlers.get(event)!.add(handler);
        return () => {
            unsubscribed.push(event);
            handlers.get(event)!.delete(handler);
        };
    },
}));
vi.mock("@/app/store/ws", () => ({
    addWSReconnectHandler: (handler: () => void) => {
        reconnect = handler;
        reconnectHandlers++;
    },
}));

import { createDomainStore } from "./domain-store";

const fire = (event: string) => handlers.get(event)?.forEach((h) => h());
const flush = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
};

/** Follows `store` the way a mounted component does; the returned function unmounts it. */
function mount(store: { follow: () => void }): () => void {
    let dispose = () => {};
    createRoot((d) => {
        dispose = d;
        store.follow();
    });
    return dispose;
}

describe("createDomainStore", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        handlers.clear();
        unsubscribed.length = 0;
    });
    afterEach(() => vi.useRealTimers());

    it("loads when first followed, and holds the result", async () => {
        const load = vi.fn().mockResolvedValue(["a"]);
        const store = createDomainStore({ initial: [] as string[], load, events: ["agents:changed" as never] });
        expect(store.loaded()).toBe(false);
        const unmount = mount(store);
        await flush();
        expect(load).toHaveBeenCalledTimes(1);
        expect(store.data()).toEqual(["a"]);
        expect(store.loaded()).toBe(true);
        unmount();
    });

    it("reloads once for a burst of events", async () => {
        const load = vi.fn().mockResolvedValue(1);
        const store = createDomainStore({ initial: 0, load, events: ["x" as never], debounceMs: 100 });
        const unmount = mount(store);
        await flush();
        load.mockClear();
        fire("x");
        fire("x");
        fire("x");
        await vi.advanceTimersByTimeAsync(99);
        expect(load).not.toHaveBeenCalled();
        await vi.advanceTimersByTimeAsync(2);
        expect(load).toHaveBeenCalledTimes(1);
        unmount();
    });

    it("an event during a load makes exactly one more load after it, never two at once", async () => {
        let release: (v: number) => void = () => {};
        let running = 0;
        let overlapped = false;
        const load = vi.fn(
            () =>
                new Promise<number>((resolve) => {
                    if (++running > 1) overlapped = true;
                    release = (v) => {
                        running--;
                        resolve(v);
                    };
                })
        );
        const store = createDomainStore({ initial: 0, load, events: ["x" as never], debounceMs: 0 });
        const unmount = mount(store);
        await flush();
        expect(load).toHaveBeenCalledTimes(1);
        fire("x");
        await vi.advanceTimersByTimeAsync(1); // the event asks while the first load runs
        expect(load).toHaveBeenCalledTimes(1);
        release(1);
        await flush();
        expect(load).toHaveBeenCalledTimes(2);
        release(2);
        await flush();
        expect(store.data()).toBe(2);
        expect(overlapped).toBe(false);
        unmount();
    });

    it("reloads every followed store after a reconnect, and not one nobody follows", async () => {
        const a = vi.fn().mockResolvedValue(1);
        const b = vi.fn().mockResolvedValue(1);
        const followedStore = createDomainStore({ initial: 0, load: a, events: ["x" as never] });
        const idle = createDomainStore({ initial: 0, load: b, events: ["y" as never] });
        const unmount = mount(followedStore);
        await flush();
        a.mockClear();
        reconnect?.();
        await flush();
        expect(a).toHaveBeenCalledTimes(1);
        expect(b).not.toHaveBeenCalled();
        void idle;
        unmount();
        a.mockClear();
        reconnect?.();
        await flush();
        expect(a).not.toHaveBeenCalled();
    });

    it("keeps one subscription for several followers and drops it with the last", async () => {
        const store = createDomainStore({ initial: 0, load: vi.fn().mockResolvedValue(1), events: ["x" as never] });
        const first = mount(store);
        const second = mount(store);
        expect(handlers.get("x")!.size).toBe(1);
        first();
        expect(unsubscribed).toEqual([]);
        second();
        expect(unsubscribed).toEqual(["x"]);
    });

    it("polls while followed when asked, and stops after", async () => {
        const load = vi.fn().mockResolvedValue(1);
        const store = createDomainStore({ initial: 0, load, events: [], safetyPollMs: 1000 });
        const unmount = mount(store);
        await flush();
        load.mockClear();
        await vi.advanceTimersByTimeAsync(2500);
        expect(load).toHaveBeenCalledTimes(2);
        unmount();
        load.mockClear();
        await vi.advanceTimersByTimeAsync(5000);
        expect(load).not.toHaveBeenCalled();
    });

    it("keeps the data and says what failed, then clears the error on the next success", async () => {
        const load = vi.fn().mockResolvedValueOnce("good").mockRejectedValueOnce(new Error("srv down")).mockResolvedValue("better");
        const store = createDomainStore({ initial: "", load, events: [] });
        const unmount = mount(store);
        await flush();
        await store.refresh();
        expect(store.data()).toBe("good");
        expect(store.error()).toBe("srv down");
        await store.refresh();
        expect(store.data()).toBe("better");
        expect(store.error()).toBeNull();
        unmount();
    });

    it("merge decides what the store holds", async () => {
        const load = vi.fn().mockResolvedValue([{ id: 1 }]);
        const merge = vi.fn((prev: Array<{ id: number }>, next: Array<{ id: number }>) => (prev.length ? prev : next));
        const store = createDomainStore({ initial: [] as Array<{ id: number }>, load, events: [], merge });
        const unmount = mount(store);
        await flush();
        const first = store.data();
        await store.refresh();
        expect(store.data()).toBe(first);
        unmount();
    });

    it("hooks the reconnect handler once for all stores", () => {
        expect(reconnectHandlers).toBe(1);
    });
});
