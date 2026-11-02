// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";
import { refetchAbsentBlocks, type BlockCacheAccess } from "./swarm-block-heal";

/** A cache that holds `absent` orefs as read and not found, and records what it is asked to read. */
function fakeCache(absent: string[], reload: (oref: string) => Promise<unknown> = async () => undefined) {
    const held = new Set(absent);
    const reloads: string[] = [];
    const cache: BlockCacheAccess = {
        isAbsent: (oref) => held.has(oref),
        reload: (oref) => {
            reloads.push(oref);
            return reload(oref);
        },
    };
    return { cache, reloads, held };
}

describe("refetchAbsentBlocks", () => {
    it("reads again a listed agent's block the window holds as missing, and only that one", async () => {
        const { cache, reloads } = fakeCache(["block:naki"]);
        expect(refetchAbsentBlocks(["camper", "naki", "winget"], cache)).toBe(1);
        expect(reloads).toEqual(["block:naki"]);
        await Promise.resolve();
    });

    it("lets the block's row come back: the read fills the cache, so the next look finds it", async () => {
        const { cache, held } = fakeCache(["block:winget"], async (oref) => void held.delete(oref));
        refetchAbsentBlocks(["winget"], cache);
        await vi.waitFor(() => expect(cache.isAbsent("block:winget")).toBe(false));
    });

    it("does not ask twice for a block whose read has not come back, and asks again once it has", async () => {
        let answer!: () => void;
        const { cache, reloads } = fakeCache(["block:naki"], () => new Promise<void>((resolve) => (answer = resolve)));
        expect(refetchAbsentBlocks(["naki"], cache)).toBe(1);
        expect(refetchAbsentBlocks(["naki"], cache)).toBe(0);
        answer();
        await vi.waitFor(() => expect(refetchAbsentBlocks(["naki"], cache)).toBe(1));
        expect(reloads).toEqual(["block:naki", "block:naki"]);
        answer();
        await Promise.resolve();
    });

    it("survives a block srv does not have either: the read rejects, nothing throws, and it is asked about again later", async () => {
        const { cache } = fakeCache(["block:gone"], () => Promise.reject(new Error("not found: block:gone")));
        expect(() => refetchAbsentBlocks(["gone"], cache)).not.toThrow();
        await vi.waitFor(() => expect(refetchAbsentBlocks(["gone"], cache)).toBe(1));
    });

    it("reads nothing when no listed agent's block is missing", () => {
        const { cache, reloads } = fakeCache([]);
        expect(refetchAbsentBlocks(["camper", "naki"], cache)).toBe(0);
        expect(reloads).toEqual([]);
    });
});
