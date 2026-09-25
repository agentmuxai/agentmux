// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({ MOS: { makeORef: (t: string, id: string) => `${t}:${id}` } }));
const subscriptions: { eventType: string; scope: string; handler: (e: { data?: unknown }) => void }[] = [];
const unsubscribe = vi.fn();
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (s: (typeof subscriptions)[number]) => {
        subscriptions.push(s);
        return unsubscribe;
    },
}));

import {
    applyShutdownEvent,
    beginShutdownLog,
    endShutdownLog,
    failShutdownLog,
    shutdownLogFor,
    visibleShutdownLines,
    type ShutdownLog,
} from "./shutdown-log";

const empty: ShutdownLog = { lines: [], done: false };
const ev = (seq: number, step: string, text: string, extra: Record<string, unknown> = {}) => ({ seq, step, text, ...extra });

describe("applyShutdownEvent", () => {
    it("orders by seq, not arrival, and ignores duplicates and junk", () => {
        let log = applyShutdownEvent(empty, ev(3, "saved", "conversation saved"));
        log = applyShutdownEvent(log, ev(1, "interrupt", "turn interrupted"));
        log = applyShutdownEvent(log, ev(1, "interrupt", "turn interrupted"));
        log = applyShutdownEvent(log, { nonsense: true });
        expect(log.lines.map((l) => l.seq)).toEqual([1, 3]);
        expect(log.done).toBe(false);
    });

    it("done marks it finished; error keeps the reason", () => {
        expect(applyShutdownEvent(empty, ev(9, "done", "closing")).done).toBe(true);
        const failed = applyShutdownEvent(empty, ev(9, "error", "couldn't close: x", { error: "x" }));
        expect(failed.error).toBe("x");
        expect(failed.done).toBe(false);
    });
});

describe("visibleShutdownLines", () => {
    it("caps at 8, keeping the newest, and counts the rest", () => {
        const lines = Array.from({ length: 11 }, (_, i) => ({ seq: i + 1, step: "process", text: `p${i + 1}` }));
        const v = visibleShutdownLines(lines);
        expect(v.more).toBe(3);
        expect(v.shown.map((l) => l.text)).toEqual(["p4", "p5", "p6", "p7", "p8", "p9", "p10", "p11"]);
        expect(visibleShutdownLines(lines.slice(0, 2)).more).toBe(0);
    });
});

describe("beginShutdownLog / endShutdownLog", () => {
    it("listens on the block's own scope, fills the log, and stops cleanly", () => {
        beginShutdownLog("b1");
        beginShutdownLog("b1"); // idempotent
        expect(subscriptions).toHaveLength(1);
        expect(subscriptions[0]).toMatchObject({ eventType: "agent:shutdown", scope: "block:b1" });
        subscriptions[0].handler({ data: ev(1, "exit", "Camper — exited") });
        expect(shutdownLogFor("b1")?.lines.map((l) => l.text)).toEqual(["Camper — exited"]);
        endShutdownLog("b1");
        expect(unsubscribe).toHaveBeenCalledTimes(1);
        expect(shutdownLogFor("b1")).toBeUndefined();
    });
});

describe("failShutdownLog (ReAgent P1 on #3784)", () => {
    it("a close rejected before srv reported anything shows the error, so the pane offers a way out", () => {
        beginShutdownLog("b2");
        failShutdownLog("b2", "ClosePane: tab not found");
        expect(shutdownLogFor("b2")?.error).toBe("ClosePane: tab not found");
        endShutdownLog("b2");
    });

    it("does nothing to a pane that isn't closing, or already finished", () => {
        failShutdownLog("never-began", "x");
        expect(shutdownLogFor("never-began")).toBeUndefined();
        beginShutdownLog("b3");
        const handler = subscriptions[subscriptions.length - 1].handler;
        handler({ data: ev(1, "done", "closing") });
        failShutdownLog("b3", "late");
        expect(shutdownLogFor("b3")?.error).toBeUndefined();
        endShutdownLog("b3");
    });
});
