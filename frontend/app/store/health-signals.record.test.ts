// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the rules that read the long-run record have flagged reaches the status bar's health store as the
 * persisted `record-flags` mux event: a `record` signal, a note that is never critical, last among the kinds.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mux = vi.hoisted(() => ({
    handlers: new Map<string, (p: unknown) => void>(),
    host: new Map<string, (p: unknown) => void>(),
}));

vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (type: string, handler: (p: unknown) => void) => {
        mux.handlers.set(type, handler);
        return () => mux.handlers.delete(type);
    },
}));
vi.mock("@/util/app-api", () => ({
    getApi: () => ({
        listen: async (event: string, handler: (p: unknown) => void) => {
            mux.host.set(event, handler);
            return () => mux.host.delete(event);
        },
        getHealthSignals: async () => [],
    }),
}));

import { WpsEvent } from "@/app/store/mps-events";
import type { RecordFlagLine } from "@/types/rpc/RecordFlagLine";
import { recordFlagText, recordMessage } from "./health-record-text";
import { applyHealthEvent, healthSignals, startHealthSignals, worstHealthLevel } from "./health-signals";
import { healthLabel, messageFor } from "./health-signals-text";

const MB = 1024 * 1024;
const flag = (over: Partial<RecordFlagLine>): RecordFlagLine => ({
    rule: "stalls",
    key: "srv_stall",
    t: 7000,
    n: null,
    worst_ms: null,
    unit: null,
    level: null,
    rise: null,
    ...over,
});
const heap = flag({ rule: "growth", key: "page:main:heap", unit: "mb", level: 306, rise: 72 });
const stalls = flag({ n: 4, worst_ms: 6200 });
const record = (flags: RecordFlagLine[]) => ({
    kind: "record" as const,
    level: flags.length ? ("warn" as const) : ("normal" as const),
    since_ms: flags.length ? Math.min(...flags.map((f) => f.t)) : null,
    flags,
});

describe("the record signal", () => {
    it("is the last of the kinds: every signal of now is above a note about days", () => {
        let s = applyHealthEvent([], record([heap]), 1);
        s = applyHealthEvent(s, { kind: "ram", level: "warn" }, 2);
        s = applyHealthEvent(s, { kind: "backend", level: "warn" }, 3);
        expect(s.map((x) => x.kind)).toEqual(["backend", "ram", "record"]);
    });

    it("starts at its oldest flag, and an empty list clears it", () => {
        const on = applyHealthEvent([], record([heap, flag({ t: 9000 })]), 100);
        expect(on[0].since).toBe(7000);
        expect(applyHealthEvent(on, record([]), 200)).toEqual([]);
    });
});

describe("the words for a flag", () => {
    it("say which part stalled, how often and the longest", () => {
        expect(recordFlagText(stalls)).toBe(
            "AgentMux's backend stopped responding 4 times for 5 s or more within one hour (the longest 6 s)."
        );
        expect(recordFlagText(flag({ key: "ui_stall", n: 5, worst_ms: 12_400 }))).toBe(
            "AgentMux's window stopped responding 5 times for 5 s or more within one hour (the longest 12 s)."
        );
        expect(recordFlagText(flag({ rule: "long_stall", key: "ui_stall", worst_ms: 41_000 }))).toBe(
            "AgentMux's window stopped responding for 41 s."
        );
        expect(recordFlagText(flag({ rule: "long_stall", worst_ms: 25_000 }))).toBe("AgentMux's backend stopped responding for 25 s.");
    });

    it("say what grew, by how much a day and to what, in what it is counted in", () => {
        expect(recordFlagText(heap)).toBe(
            "Memory in one window has grown faster than its content for two days: about 72 MB a day, now 306 MB."
        );
        expect(recordFlagText(flag({ rule: "growth", key: "page:main:dom", unit: "elements", level: 52_000, rise: 12_000 }))).toBe(
            "One window's page has grown faster than its content for two days: about 12,000 elements a day, now 52,000 elements."
        );
        expect(recordFlagText(flag({ rule: "growth", key: "core:srv", unit: "bytes", level: 300 * MB, rise: 90 * MB }))).toBe(
            "AgentMux's own memory has grown for two days: about 90 MB a day, now 300 MB."
        );
    });

    it("leave out a rule this build does not know, and always say that nothing was stopped", () => {
        const tail = "Nothing was stopped. The record in the Tower has the days behind this.";
        expect(recordFlagText(flag({ rule: "later_rule" }))).toBe("");
        expect(recordMessage([flag({ rule: "later_rule" })])).toBe(tail);
        expect(recordMessage(undefined)).toBe(tail);
        expect(recordMessage([stalls, heap])).toBe(`${recordFlagText(stalls)} ${recordFlagText(heap)} ${tail}`);
    });

    it("are what the notice shows", () => {
        expect(healthLabel("record")).toBe("Long-run record");
        expect(messageFor("record", "warn", record([heap]))).toBe(recordMessage([heap]));
    });
});

describe("startHealthSignals follows record-flags", () => {
    let stop: () => void;
    beforeEach(() => {
        mux.handlers.clear();
        stop = startHealthSignals();
    });
    afterEach(() => stop());

    it("shows a note while a flag is said, and none when the list is empty", () => {
        const say = mux.handlers.get(WpsEvent.RecordFlags)!;
        expect(worstHealthLevel()).toBe("normal");
        say(record([stalls]));
        expect(worstHealthLevel()).toBe("warn");
        expect(healthSignals()[0].payload.flags).toEqual([stalls]);
        say(record([stalls, heap]));
        expect(healthSignals()).toHaveLength(1);
        say(record([]));
        expect(healthSignals()).toEqual([]);
    });

    it("drops the note when the backend is restarted: the new one says it again if it still holds", async () => {
        await Promise.resolve();
        const say = mux.handlers.get(WpsEvent.RecordFlags)!;
        mux.host.get("health-signal")!({ kind: "disk", level: "warn" });
        say(record([heap]));
        expect(healthSignals().map((s) => s.kind)).toEqual(["disk", "record"]);
        mux.host.get("backend-ready")!({});
        expect(healthSignals().map((s) => s.kind)).toEqual(["disk"]);
        mux.host.get("health-signal")!({ kind: "disk", level: "normal" });
    });

    it("stops listening when stopped", () => {
        stop();
        expect(mux.handlers.size).toBe(0);
    });
});
