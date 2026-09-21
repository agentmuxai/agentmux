// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression coverage for `getMuxObjectAtom` surviving cache eviction.
 *
 * `cleanMuxObjectCache` runs on a 30s interval and drops any entry whose
 * `refCount` is 0 once its holdTime lapses. `getMuxObjectAtom` takes no
 * refCount (it has no cleanup hook), so its entries are always eviction
 * candidates. When the atom captured its cache entry once, eviction silently
 * froze every reader on the pre-eviction value forever — `updateMuxObject`
 * would re-resolve the oref, create a fresh entry, and write there.
 *
 * Observed as: Ctrl+Wheel zoom doing nothing in the agent Shell drawer, whose
 * headless sub-block has no other refCount holder.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createEffect, createRoot } from "solid-js";

vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { MuxObjUpdate: "waveobj:update" } }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://localhost" }));
vi.mock("@/util/fetchutil", () => ({ fetch: () => new Promise(() => {}) }));
vi.mock("./services", () => ({ ObjectService: { UpdateObject: async () => {} } }));
vi.mock("./app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));

/** A block-shaped MuxObj at a given version. */
function block(oid: string, version: number, zoom?: number) {
    return {
        otype: "block",
        oid,
        version,
        meta: zoom == null ? {} : { "term:zoom": zoom },
    } as any;
}

describe("getMuxObjectAtom — survives cache eviction", () => {
    beforeEach(() => {
        vi.resetModules();
    });

    it("reflects an update that lands after the cache entry was evicted", async () => {
        vi.useFakeTimers();
        const MOS = await import("./mos");
        const oref = "block:sub-1";

        const atom = MOS.getMuxObjectAtom(oref);

        // Seed a value, as a backend push would.
        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "sub-1", obj: block("sub-1", 1, 1.0) } as any);
        expect((atom() as any)?.meta?.["term:zoom"]).toBe(1.0);

        // Let the GC interval fire with refCount still 0 — the entry is dropped.
        vi.advanceTimersByTime(60_000);

        // A later push (e.g. the zoom write echoing back) must still be visible.
        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "sub-1", obj: block("sub-1", 2, 1.5) } as any);

        expect((atom() as any)?.meta?.["term:zoom"]).toBe(1.5);
        vi.useRealTimers();
    });

    it("keeps reflecting updates with no eviction in between", async () => {
        const MOS = await import("./mos");
        const oref = "block:sub-2";
        const atom = MOS.getMuxObjectAtom(oref);

        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "sub-2", obj: block("sub-2", 1, 1.0) } as any);
        expect((atom() as any)?.meta?.["term:zoom"]).toBe(1.0);

        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "sub-2", obj: block("sub-2", 2, 1.2) } as any);
        expect((atom() as any)?.meta?.["term:zoom"]).toBe(1.2);
    });

    /**
     * The one that actually matches the reported symptom. Reading the fresh
     * value is not enough — a dependent effect has to RE-RUN. Once an entry is
     * evicted and re-created, the new entry carries a brand new signal, and
     * anything still subscribed to the old one is never re-triggered. So the
     * entry must stay alive while a reactive owner depends on it.
     */
    it("re-runs a dependent effect after a GC tick (the drawer-zoom symptom)", async () => {
        vi.useFakeTimers();
        const MOS = await import("./mos");
        const seen: (number | undefined)[] = [];

        const dispose = createRoot((disposeFn) => {
            const atom = MOS.getMuxObjectAtom("block:sub-live");
            createEffect(() => {
                seen.push((atom() as any)?.meta?.["term:zoom"]);
            });
            return disposeFn;
        });

        // Let the effect's first run flush.
        await Promise.resolve();
        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "sub-live", obj: block("sub-live", 1, 1.0) } as any);
        await Promise.resolve();

        // GC tick with the entry pinned by the live owner — must NOT be evicted.
        vi.advanceTimersByTime(60_000);

        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "sub-live", obj: block("sub-live", 2, 1.5) } as any);
        await Promise.resolve();

        // The post-eviction-window update must have reached the effect.
        expect(seen).toContain(1.5);
        dispose();
        vi.useRealTimers();
    });

    it("releases the pin when the owner is disposed, so the entry can still be collected", async () => {
        vi.useFakeTimers();
        const MOS = await import("./mos");
        const dispose = createRoot((disposeFn) => {
            MOS.getMuxObjectAtom("block:sub-temp");
            return disposeFn;
        });
        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "sub-temp", obj: block("sub-temp", 1, 1.0) } as any);

        dispose(); // refCount back to 0

        // With no owner holding it, the GC is free to reclaim it — proven by the
        // re-created entry starting empty rather than serving the stale value.
        vi.advanceTimersByTime(60_000);
        const fresh = MOS.getMuxObjectAtom("block:sub-temp");
        expect((fresh() as any)?.meta?.["term:zoom"]).toBeUndefined();
        vi.useRealTimers();
    });

    it("ignores a stale update whose version is not newer", async () => {
        const MOS = await import("./mos");
        const atom = MOS.getMuxObjectAtom("block:sub-3");

        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "sub-3", obj: block("sub-3", 5, 2.0) } as any);
        MOS.updateMuxObject({ updatetype: "update", otype: "block", oid: "sub-3", obj: block("sub-3", 4, 0.5) } as any);

        expect((atom() as any)?.meta?.["term:zoom"]).toBe(2.0);
    });
});

describe("getMuxObjectCacheStats", () => {
    beforeEach(() => {
        vi.resetModules();
    });

    it("counts a pinned entry (live reactive owner) separately from an unpinned one", async () => {
        const MOS = await import("./mos");

        // Unpinned: getMuxObjectAtom called with no reactive owner in scope.
        // The atom is lazy — creation only happens on first read.
        MOS.getMuxObjectAtom("block:stats-unpinned")();

        const dispose = createRoot((disposeFn) => {
            MOS.getMuxObjectAtom("block:stats-pinned");
            return disposeFn;
        });

        const stats = MOS.getMuxObjectCacheStats();
        expect(stats.totalEntries).toBe(2);
        expect(stats.pinnedEntries).toBe(1);
        expect(stats.totalRefCount).toBe(1);

        dispose();
    });

    it("reflects refCount dropping back to zero after the owner disposes", async () => {
        const MOS = await import("./mos");
        const dispose = createRoot((disposeFn) => {
            MOS.getMuxObjectAtom("block:stats-dispose");
            return disposeFn;
        });
        expect(MOS.getMuxObjectCacheStats().pinnedEntries).toBe(1);

        dispose();
        expect(MOS.getMuxObjectCacheStats().pinnedEntries).toBe(0);
        // Entry itself is still cached (only the periodic sweep evicts it) —
        // this getter must not mutate the cache as a side effect of reading it.
        expect(MOS.getMuxObjectCacheStats().totalEntries).toBe(1);
    });

    it("sums refCount across multiple owners pinning the same oref", async () => {
        const MOS = await import("./mos");
        const disposeA = createRoot((disposeFn) => {
            MOS.getMuxObjectAtom("block:stats-shared");
            return disposeFn;
        });
        const disposeB = createRoot((disposeFn) => {
            MOS.getMuxObjectAtom("block:stats-shared");
            return disposeFn;
        });

        const stats = MOS.getMuxObjectCacheStats();
        expect(stats.totalEntries).toBe(1);
        expect(stats.pinnedEntries).toBe(1);
        expect(stats.totalRefCount).toBe(2);

        disposeA();
        expect(MOS.getMuxObjectCacheStats().totalRefCount).toBe(1);
        disposeB();
        expect(MOS.getMuxObjectCacheStats().totalRefCount).toBe(0);
    });
});
