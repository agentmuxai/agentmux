// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";

const handlers: Array<(event: unknown) => void> = [];
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (sub: { handler: (event: unknown) => void }) => {
        handlers.push(sub.handler);
        return () => {};
    },
}));

import { cachedTurnLedger, useTurnLedger } from "./useTurnLedger";

const payload = (turnId: number, passes: number) => ({
    data: { turn_id: turnId, started_at_ms: turnId, active: true, passes },
});

function mount(blockId: string) {
    const dispatched: Array<{ type: string; ledger?: { turnId: number; passes: number } }> = [];
    let dispose = () => {};
    createRoot((d) => {
        dispose = d;
        useTurnLedger(blockId, { dispatchPane: (c: any) => void dispatched.push(c) } as any);
    });
    return { dispatched, dispose };
}

describe("useTurnLedger", () => {
    it("re-feeds the latest ledger to a remounted pane, which gets no replay (#4492)", async () => {
        const first = mount("blk-remount");
        await Promise.resolve();
        handlers.at(-1)!(payload(500, 2));
        expect(first.dispatched.at(-1)?.ledger).toMatchObject({ turnId: 500, passes: 2 });
        first.dispose();

        const second = mount("blk-remount");
        await Promise.resolve();
        expect(second.dispatched[0]?.ledger).toMatchObject({ turnId: 500, passes: 2 });
        second.dispose();
    });

    it("keeps the newest turn when an older one arrives late", async () => {
        const m = mount("blk-order");
        await Promise.resolve();
        handlers.at(-1)!(payload(900, 1));
        handlers.at(-1)!(payload(800, 3));
        expect(cachedTurnLedger("blk-order")?.turnId).toBe(900);
        m.dispose();
    });
});
