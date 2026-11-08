// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";

type Handler = (data: { values?: Record<string, number> }) => void;
const subs: { scope: string; handler: Handler; stopped: boolean }[] = [];
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_type: string, handler: Handler, scope: string) => {
        const sub = { scope, handler, stopped: false };
        subs.push(sub);
        return () => (sub.stopped = true);
    },
}));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { BlockStats: "blockstats" } }));

import { useBlockStats } from "./useBlockStats";

const send = (i: number, values: Record<string, number>) => subs[i].handler({ values });

describe("useBlockStats", () => {
    it("shares one subscription between readers of a pane, and a reader that comes late has the last figures at once", () => {
        subs.length = 0;
        createRoot((dispose) => {
            const first = useBlockStats("b1");
            expect(first()).toBeNull();
            send(0, { cpu: 12, mem: 100, "mem:peak": 150, pids: 2 });
            expect(first()).toEqual({ cpu: 12, mem: 100, peak: 150, pids: 2 });

            const late = useBlockStats("b1");
            expect(subs).toHaveLength(1);
            expect(subs[0].scope).toBe("block:b1");
            expect(late()).toEqual({ cpu: 12, mem: 100, peak: 150, pids: 2 });
            send(0, { cpu: 3, mem: 90, pids: 1 });
            expect(late()).toEqual({ cpu: 3, mem: 90, peak: 90, pids: 1 });
            expect(first()).toBe(late());
            dispose();
        });
    });

    it("keeps one pane's figures apart from another's", () => {
        subs.length = 0;
        createRoot((dispose) => {
            const a = useBlockStats("a");
            const b = useBlockStats("b");
            expect(subs.map((s) => s.scope)).toEqual(["block:a", "block:b"]);
            send(0, { cpu: 1, mem: 1, pids: 1 });
            expect(a()?.cpu).toBe(1);
            expect(b()).toBeNull();
            dispose();
        });
    });

    it("listens until the last reader goes, then forgets the figures", () => {
        subs.length = 0;
        const [a, disposeA] = createRoot((d) => [useBlockStats("b1"), d] as const);
        const disposeB = createRoot((d) => {
            useBlockStats("b1");
            return d;
        });
        send(0, { cpu: 5, mem: 1, pids: 1 });
        disposeA();
        expect(subs[0].stopped).toBe(false);
        expect(a()?.cpu).toBe(5);
        disposeB();
        expect(subs[0].stopped).toBe(true);

        createRoot((dispose) => {
            const again = useBlockStats("b1");
            expect(subs).toHaveLength(2);
            expect(again()).toBeNull();
            dispose();
        });
    });
});
