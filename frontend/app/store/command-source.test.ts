// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import { createEffect, createRoot, createSignal } from "solid-js";
import {
    __resetDispatchLog,
    describeSource,
    dispatchRecordsAtom,
    getRecentDispatches,
    recordDispatch,
} from "./command-source";

describe("command-source / dispatch log", () => {
    beforeEach(() => {
        __resetDispatchLog();
    });

    it("records appended in order", () => {
        recordDispatch({
            slice: "agent-document",
            key: "block-a",
            command: { type: "X" },
            events: [],
            source: "user",
            at: 100,
        });
        recordDispatch({
            slice: "agent-document",
            key: "block-b",
            command: { type: "Y" },
            events: [],
            source: "system",
            at: 200,
        });
        const all = getRecentDispatches();
        expect(all).toHaveLength(2);
        expect(all[0].at).toBe(100);
        expect(all[1].at).toBe(200);
    });

    it("trims to ring capacity (500)", () => {
        for (let i = 0; i < 600; i++) {
            recordDispatch({
                slice: "test",
                key: null,
                command: { type: "X", i },
                events: [],
                source: "system",
                at: i,
            });
        }
        const all = getRecentDispatches();
        expect(all).toHaveLength(500);
        // Oldest 100 dropped
        expect((all[0].command as any).i).toBe(100);
        expect((all[499].command as any).i).toBe(599);
    });

    it("getRecentDispatches respects limit", () => {
        for (let i = 0; i < 5; i++) {
            recordDispatch({
                slice: "test",
                key: null,
                command: { type: "X", i },
                events: [],
                source: "system",
                at: i,
            });
        }
        const last3 = getRecentDispatches(3);
        expect(last3.map((r) => (r.command as any).i)).toEqual([2, 3, 4]);
    });

    it("dispatchRecordsAtom reflects writes reactively", () => {
        expect(dispatchRecordsAtom()).toHaveLength(0);
        recordDispatch({
            slice: "test",
            key: null,
            command: {},
            events: [],
            source: "system",
            at: 1,
        });
        expect(dispatchRecordsAtom()).toHaveLength(1);
    });

    // Regression: storm-crash root cause — `recordDispatch` called from
    // inside a SolidJS reactive context (e.g. the launcher-event reducer's
    // createEffect) must NOT establish a dependency on `recordsAtom`.
    // Without `untrack`, the read+write pair on recordsAtom caused the
    // outer effect to re-run on every dispatch, observed as ~3000×
    // runaway and renderer V8-stack crash on the storm path.
    it("does not register reactive deps when called from inside createEffect", async () => {
        let outerEffectRunCount = 0;
        const dispose = createRoot((d) => {
            const [trigger, setTrigger] = createSignal(0);
            createEffect(() => {
                trigger();
                outerEffectRunCount++;
                recordDispatch({
                    slice: "test",
                    key: null,
                    command: { type: "X" },
                    events: [],
                    source: "system",
                    at: outerEffectRunCount,
                });
            });
            // SolidJS schedules effects on microtask; flush synchronously
            // by triggering the signal a 2nd time after the first commit.
            queueMicrotask(() => setTrigger((n) => n + 1));
            return d;
        });
        // Wait for both microtask + any subsequent ones (if there's a
        // leak the count would balloon; bound the wait by yielding twice).
        await Promise.resolve();
        await Promise.resolve();
        await new Promise((r) => setTimeout(r, 0));
        dispose();
        // Initial run + one explicit trigger = 2 runs total. Anything
        // more means recordDispatch leaked a reactive dep on recordsAtom
        // (the storm-crash root cause).
        expect(outerEffectRunCount).toBe(2);
    });

    it("describeSource handles all variants", () => {
        expect(describeSource("system")).toBe("system");
        expect(describeSource("user")).toBe("user");
        expect(describeSource({ kind: "agent", agentId: "agent-1" })).toBe("agent:agent-1");
    });
});
