// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, createRoot } from "solid-js";
import { describe, expect, it } from "vitest";
import { createKeyedRegistry } from "./keyed-registry";

describe("createKeyedRegistry", () => {
    it("registers, looks up, and lists", () => {
        const reg = createKeyedRegistry<{ n: number }>();
        const a = { n: 1 };
        const off = reg.register("a", a);
        reg.register("b", { n: 2 });
        expect(reg.get("a")).toBe(a);
        expect(reg.all().map((v) => v.n)).toEqual([1, 2]);
        off();
        expect(reg.get("a")).toBeUndefined();
    });

    // A model rebuilt for the same block (host rule 8's per-instance root)
    // registers before the old one's dispose runs; the old unregister must
    // not remove the new model.
    it("a stale unregister leaves a newer registration alone", () => {
        const reg = createKeyedRegistry<string>();
        const offOld = reg.register("blk", "old");
        reg.register("blk", "new");
        offOld();
        expect(reg.get("blk")).toBe("new");
    });

    it("is reactive: a memo re-runs when its key registers", () => {
        createRoot((dispose) => {
            const reg = createKeyedRegistry<string>();
            const seen = createMemo(() => reg.get("blk") ?? null);
            expect(seen()).toBeNull();
            reg.register("blk", "model");
            expect(seen()).toBe("model");
            dispose();
        });
    });
});
