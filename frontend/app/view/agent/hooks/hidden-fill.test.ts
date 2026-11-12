// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { createHiddenFill, PRESSURE_RECHECK_MS, type HiddenFillPressure } from "./hidden-fill";

function make(initial: HiddenFillPressure = "normal") {
    let now = 0;
    let answer: HiddenFillPressure = initial;
    let asked = 0;
    const gate = createHiddenFill({
        pressure: async () => {
            asked++;
            return answer;
        },
        now: () => now,
    });
    return {
        gate,
        advance: (ms: number) => (now += ms),
        set: (p: HiddenFillPressure) => (answer = p),
        asked: () => asked,
    };
}

describe("a hidden pane's fill", () => {
    it("is allowed until the page says it is under memory pressure, asking at most every few seconds", async () => {
        const m = make();
        expect(m.gate.allowed()).toBe(true);
        expect(m.gate.allowed()).toBe(true);
        expect(m.asked()).toBe(1);
        m.set("warn");
        m.advance(PRESSURE_RECHECK_MS - 1);
        m.gate.allowed();
        expect(m.asked()).toBe(1);
        m.advance(1);
        m.gate.allowed(); // asks; the answer arrives after this call
        await Promise.resolve();
        await Promise.resolve();
        expect(m.gate.allowed()).toBe(false);
        m.set("normal");
        m.advance(PRESSURE_RECHECK_MS);
        m.gate.allowed();
        await Promise.resolve();
        await Promise.resolve();
        expect(m.gate.allowed()).toBe(true);
    });

    it("stays allowed when the page cannot say", async () => {
        const gate = createHiddenFill({ pressure: () => Promise.reject(new Error("no")), now: () => 0 });
        gate.allowed();
        await Promise.resolve();
        await Promise.resolve();
        expect(gate.allowed()).toBe(true);
    });

    it("runs one pane's page at a time, in the order asked, and a failed one does not stop the rest", async () => {
        const { gate } = make();
        const order: string[] = [];
        let release!: () => void;
        const first = gate.run(
            () =>
                new Promise<string>((resolve) => {
                    order.push("a:start");
                    release = () => {
                        order.push("a:end");
                        resolve("a");
                    };
                }),
        );
        const second = gate.run(async () => {
            order.push("b:start");
            throw new Error("read failed");
        });
        const third = gate.run(async () => {
            order.push("c:start");
            return "c";
        });
        await Promise.resolve();
        expect(order).toEqual(["a:start"]);
        release();
        expect(await first).toBe("a");
        await expect(second).rejects.toThrow("read failed");
        expect(await third).toBe("c");
        expect(order).toEqual(["a:start", "a:end", "b:start", "c:start"]);
    });
});
