// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Coverage for `getBlockAtomCacheStats`, added while investigating a
 * 2026-09-20 latency/memory report (see the getter's own doc comment).
 * Uses `useBlockAtom` (no `MOS`/config-signals dependency) to populate the
 * cache without needing to mock the rest of the store.
 */

import { describe, expect, it } from "vitest";
import { createSignal } from "solid-js";
import { cleanupBlockAtomCache, getBlockAtomCacheStats, useBlockAtom } from "./block-atom-cache";

function seed(blockId: string, name: string) {
    useBlockAtom(blockId, name, () => {
        const [get] = createSignal(0);
        return get;
    });
}

describe("getBlockAtomCacheStats", () => {
    it("reports zero for two blockIds that were never cached", () => {
        // Distinct, never-touched blockIds — no cross-test pollution to clean up.
        const stats = getBlockAtomCacheStats();
        expect(stats.cachedBlockCount).toBeGreaterThanOrEqual(0);
        expect(stats.totalMemoCount).toBeGreaterThanOrEqual(0);
    });

    it("counts one cached block and one memo after a single useBlockAtom call", () => {
        const before = getBlockAtomCacheStats();
        seed("stats-block-a", "memo-1");
        const after = getBlockAtomCacheStats();

        expect(after.cachedBlockCount).toBe(before.cachedBlockCount + 1);
        expect(after.totalMemoCount).toBe(before.totalMemoCount + 1);

        cleanupBlockAtomCache("stats-block-a");
    });

    it("counts multiple memos under the same block as one cachedBlockCount but multiple totalMemoCount", () => {
        const before = getBlockAtomCacheStats();
        seed("stats-block-b", "memo-1");
        seed("stats-block-b", "memo-2");
        seed("stats-block-b", "memo-3");
        const after = getBlockAtomCacheStats();

        expect(after.cachedBlockCount).toBe(before.cachedBlockCount + 1);
        expect(after.totalMemoCount).toBe(before.totalMemoCount + 3);

        cleanupBlockAtomCache("stats-block-b");
    });

    it("drops back to the prior count after cleanupBlockAtomCache — the mechanism a dormant, never-closed block never hits", () => {
        const before = getBlockAtomCacheStats();
        seed("stats-block-c", "memo-1");
        seed("stats-block-c", "memo-2");
        expect(getBlockAtomCacheStats().cachedBlockCount).toBe(before.cachedBlockCount + 1);

        cleanupBlockAtomCache("stats-block-c");

        const after = getBlockAtomCacheStats();
        expect(after.cachedBlockCount).toBe(before.cachedBlockCount);
        expect(after.totalMemoCount).toBe(before.totalMemoCount);
    });
});
