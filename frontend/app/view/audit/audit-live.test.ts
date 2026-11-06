// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/store/ws", () => ({ addWSReconnectHandler: () => {} }));

import { serial } from "./audit-live";

describe("serial", () => {
    it("runs again once, not once per call, when it is called while it runs", async () => {
        let release!: () => void;
        let runs = 0;
        const run = serial(async () => {
            runs++;
            if (runs === 1) await new Promise<void>((r) => (release = r));
        });

        const first = run();
        void run();
        void run();
        void run();
        expect(runs).toBe(1);
        release();
        await first;
        expect(runs).toBe(2);
    });

    it("runs again for a call after it ended", async () => {
        const fn = vi.fn(async () => {});
        const run = serial(fn);
        await run();
        await run();
        expect(fn).toHaveBeenCalledTimes(2);
    });
});
