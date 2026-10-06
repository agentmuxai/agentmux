// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ContextReading } from "@/app/store/agent-pane-state/context-reading";

const setBlockMeta = vi.hoisted(() => vi.fn<(blockId: string, meta: Record<string, unknown>) => Promise<void>>(async () => {}));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta }));
// The `agentcontextusage` subscription: the test plays srv's part.
const usageHandlers = vi.hoisted(() => [] as ((event: unknown) => void)[]);
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (sub: { eventType: string; handler: (event: unknown) => void }) => {
        if (sub.eventType === "agentcontextusage") usageHandlers.push(sub.handler);
        return () => {};
    },
}));

import { useContextReading } from "./useContextReading";

const live = (over: Partial<ContextReading> = {}): ContextReading => ({
    tokens: 300_000,
    model: "claude-sonnet-5-5",
    window: 1_000_000,
    windowSource: "reported",
    source: "live",
    at: 1,
    switchedTo: null,
    ...over,
});

const written = (r: ContextReading | null) => ["blk-1", { "agent:context": r, "term:ctx-tokens": null }];

/** Let queued meta writes go out. */
const flush = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
};

describe("useContextReading", () => {
    let dispose: (() => void) | undefined;
    let onModelSwitched: ReturnType<typeof vi.fn<(model: string) => void>>;

    beforeEach(() => {
        usageHandlers.length = 0;
        setBlockMeta.mockReset();
        setBlockMeta.mockImplementation(async () => {});
        onModelSwitched = vi.fn<(model: string) => void>();
    });
    afterEach(() => dispose?.());

    const mount = (initial: ContextReading | null, opts: { ready?: boolean; meta?: MetaType | null } = {}) => {
        const [context, setContext] = createSignal<ContextReading | null>(initial);
        const [ready, setReady] = createSignal(opts.ready ?? true);
        const [meta, setMeta] = createSignal<MetaType | null | undefined>(opts.meta ?? null);
        const meter = createRoot((d) => {
            dispose = d;
            return useContextReading("blk-1", context, { ready, meta, onModelSwitched });
        });
        return { meter, setContext, setReady, setMeta };
    };

    it("shows a plausible reading with its note, and mirrors it to meta (clearing the legacy key)", async () => {
        const { meter } = mount(live());
        expect(meter.reading()).toEqual(live());
        expect(meter.note()).toBe("Window reported by the CLI for claude-sonnet-5-5.");
        await flush();
        expect(setBlockMeta).toHaveBeenLastCalledWith(...written(live()));
    });

    it("shows nothing — and mirrors nothing — for a reading that can't be true", async () => {
        const { meter } = mount(live({ tokens: 17_000_000, window: 200_000, windowSource: "model" }));
        expect(meter.reading()).toBeNull();
        expect(meter.note()).toBeUndefined();
        await flush();
        // Meta holds no reading either, so there is nothing to write.
        expect(setBlockMeta).not.toHaveBeenCalled();
    });

    it("re-writes meta only when the reading changes by value", async () => {
        const { setContext } = mount(live());
        await flush();
        expect(setBlockMeta).toHaveBeenCalledTimes(1);
        setContext(live()); // a new object, same reading
        await flush();
        expect(setBlockMeta).toHaveBeenCalledTimes(1);
        setContext(live({ tokens: 310_000, at: 2 }));
        await flush();
        expect(setBlockMeta).toHaveBeenCalledTimes(2);
        expect(setBlockMeta).toHaveBeenLastCalledWith(...written(live({ tokens: 310_000, at: 2 })));
    });

    it("doesn't clear meta until the history restore has completed", async () => {
        // Mirroring the mount-time null would wipe the persisted reading the
        // Swarm shows, then write it back once the history seed lands.
        const persisted = live({ source: "history" });
        const { setContext, setReady } = mount(null, { ready: false, meta: { "agent:context": persisted } as MetaType });
        await flush();
        expect(setBlockMeta).not.toHaveBeenCalled();
        setContext(persisted);
        setReady(true);
        await flush();
        // The seeded reading equals what's persisted: nothing to write.
        expect(setBlockMeta).not.toHaveBeenCalled();
    });

    it("writes a real reading even before the restore completes", async () => {
        const { setContext } = mount(null, { ready: false, meta: { "agent:context": live() } as MetaType });
        setContext(live({ tokens: 320_000, at: 9 }));
        await flush();
        expect(setBlockMeta).toHaveBeenCalledExactlyOnceWith(...written(live({ tokens: 320_000, at: 9 })));
    });

    it("clears the persisted reading once the restore completes with nothing to show", async () => {
        const { setReady } = mount(null, { ready: false, meta: { "agent:context": live() } as MetaType });
        setReady(true);
        await flush();
        expect(setBlockMeta).toHaveBeenCalledExactlyOnceWith(...written(null));
    });

    it("after a failed restore, leaves the persisted reading until this pane has one of its own", async () => {
        // `ready` stays false for InitFailed.
        const { setContext } = mount(null, { ready: false, meta: { "agent:context": live() } as MetaType });
        await flush();
        expect(setBlockMeta).not.toHaveBeenCalled();
        setContext(live({ tokens: 5_000, at: 3 }));
        await flush();
        expect(setBlockMeta).toHaveBeenCalledTimes(1);
        // Once it has mirrored one, an invalidation is mirrored too.
        setContext(null);
        await flush();
        expect(setBlockMeta).toHaveBeenLastCalledWith(...written(null));
    });

    it("clears an implausible persisted reading and the legacy key", async () => {
        mount(null, { meta: { "agent:context": live({ tokens: 17_000_000, window: 200_000 }) } as MetaType });
        await flush();
        expect(setBlockMeta).toHaveBeenCalledExactlyOnceWith(...written(null));
        dispose?.();
        setBlockMeta.mockClear();
        mount(null, { meta: { "term:ctx-tokens": 17_000_000 } as MetaType });
        await flush();
        expect(setBlockMeta).toHaveBeenCalledExactlyOnceWith(...written(null));
    });

    it("writes nothing when meta already matches, and doesn't re-run on its own meta write", async () => {
        const { setMeta, setContext } = mount(live(), { meta: { "agent:context": live() } as MetaType });
        await flush();
        expect(setBlockMeta).not.toHaveBeenCalled();
        setContext(live({ tokens: 310_000, at: 2 }));
        await flush();
        expect(setBlockMeta).toHaveBeenCalledTimes(1);
        setMeta({ "agent:context": live({ tokens: 310_000, at: 2 }) } as MetaType);
        await flush();
        expect(setBlockMeta).toHaveBeenCalledTimes(1);
    });

    it("sends one write at a time, latest wins: an older reading can't land after a newer one", async () => {
        let release: () => void = () => {};
        setBlockMeta.mockImplementation(() => new Promise<void>((resolve) => (release = resolve)));
        const { setContext } = mount(live({ tokens: 100_000, at: 1 }));
        expect(setBlockMeta).toHaveBeenCalledTimes(1);
        // Two more readings while the first write is out: only the last is sent.
        setContext(live({ tokens: 110_000, at: 2 }));
        setContext(live({ tokens: 120_000, at: 3 }));
        await flush();
        expect(setBlockMeta).toHaveBeenCalledTimes(1);
        release();
        await flush();
        expect(setBlockMeta).toHaveBeenCalledTimes(2);
        expect(setBlockMeta).toHaveBeenLastCalledWith(...written(live({ tokens: 120_000, at: 3 })));
        release();
        await flush();
        expect(setBlockMeta).toHaveBeenCalledTimes(2);
    });

    it("keeps mirroring after a write fails", async () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        setBlockMeta.mockImplementationOnce(async () => {
            throw new Error("rpc down");
        });
        const { setContext } = mount(live());
        await flush();
        setContext(live({ tokens: 310_000, at: 2 }));
        await flush();
        expect(setBlockMeta).toHaveBeenLastCalledWith(...written(live({ tokens: 310_000, at: 2 })));
        expect(warn).toHaveBeenCalled();
        warn.mockRestore();
    });

    it("reports a change of the model setting, not the setting at mount", async () => {
        const { setMeta } = mount(live(), { meta: { "agent:runtime": { model: "sonnet" } } as MetaType });
        expect(onModelSwitched).not.toHaveBeenCalled();
        setMeta({ "agent:runtime": { model: "sonnet" }, "agent:context": live() } as MetaType);
        expect(onModelSwitched).not.toHaveBeenCalled();
        setMeta({ "agent:runtime": { model: "haiku" } } as MetaType);
        expect(onModelSwitched).toHaveBeenCalledExactlyOnceWith("haiku");
    });

    it("counts down to the CLI's reported auto-compact point once srv publishes it", () => {
        const { meter } = mount(live());
        expect(meter.autoCompact()).toEqual({ kind: "at", tokens: 967_000, source: "assumed" });
        for (const h of usageHandlers) {
            h({ data: { model: "claude-sonnet-5-5", auto_compact_window: 300_000, auto_compact_threshold: 267_000, auto_compact_enabled: true } });
        }
        expect(meter.autoCompact()).toEqual({ kind: "at", tokens: 267_000, source: "reported" });
        for (const h of usageHandlers) h({ data: { model: "claude-sonnet-5-5", auto_compact_enabled: false } });
        expect(meter.autoCompact()).toEqual({ kind: "off" });
    });
});
