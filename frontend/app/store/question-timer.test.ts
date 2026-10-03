// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one question timer: its states, its activity rule, its expiry, what it
 * publishes to block meta for other windows, and how a reader treats a stale
 * published copy.
 * docs/specs/SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md §2, §6.
 */

import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const writes: Array<{ oref: string; meta: Record<string, unknown> }> = [];
const [metaByBlock, setMetaByBlock] = createSignal<Record<string, Record<string, unknown>>>({});

vi.mock("@/app/store/services", () => ({
    ObjectService: {
        UpdateObjectMeta: (oref: string, meta: Record<string, unknown>) => {
            writes.push({ oref, meta });
            return Promise.resolve();
        },
    },
}));
vi.mock("@/app/store/mos", () => ({ makeORef: (kind: string, id: string) => `${kind}:${id}` }));
vi.mock("@/app/store/global", () => ({
    MOS: {
        getMuxObjectAtom: (oref: string) => () => {
            const meta = metaByBlock()[oref.replace(/^block:/, "")];
            return meta ? { meta } : undefined;
        },
    },
}));

import {
    countdownBand,
    endQuestionTimer,
    META_QUESTION_TIMER,
    noteQuestionActivity,
    questionCountdown,
    questionTimer,
    resetQuestionTimersForTests,
    setQuestionTimerDormant,
    startQuestionTimer,
} from "./question-timer";

const published = () => writes.map((w) => w.meta[META_QUESTION_TIMER]);

beforeEach(() => {
    vi.useFakeTimers();
    writes.length = 0;
    setMetaByBlock({});
});

afterEach(() => {
    resetQuestionTimersForTests();
    vi.useRealTimers();
});

describe("question timer: owner", () => {
    it("starts counting with endsAt and fires onExpire exactly once at the deadline", () => {
        const onExpire = vi.fn();
        startQuestionTimer("b1", { durationMs: 30_000, onExpire });
        expect(questionTimer("b1")).toEqual({ kind: "counting", endsAt: Date.now() + 30_000 });

        vi.advanceTimersByTime(29_999);
        expect(onExpire).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(onExpire).toHaveBeenCalledTimes(1);
        expect(questionTimer("b1")).toBeNull();

        vi.advanceTimersByTime(60_000);
        expect(onExpire).toHaveBeenCalledTimes(1);
    });

    it("activity pauses, and the full duration resumes 15s after the LAST activity", () => {
        const onExpire = vi.fn();
        startQuestionTimer("b1", { durationMs: 30_000, onExpire });
        vi.advanceTimersByTime(20_000);
        noteQuestionActivity("b1");
        expect(questionTimer("b1")).toEqual({ kind: "paused", reason: "activity" });

        vi.advanceTimersByTime(10_000);
        noteQuestionActivity("b1"); // restarts the quiet window
        vi.advanceTimersByTime(14_999);
        expect(questionTimer("b1")).toEqual({ kind: "paused", reason: "activity" });
        vi.advanceTimersByTime(1);
        expect(questionTimer("b1")).toEqual({ kind: "counting", endsAt: Date.now() + 30_000 });

        vi.advanceTimersByTime(30_000);
        expect(onExpire).toHaveBeenCalledTimes(1);
    });

    it("dormant pauses without limit, and waking re-arms at the full duration", () => {
        const onExpire = vi.fn();
        startQuestionTimer("b1", { durationMs: 30_000, onExpire });
        setQuestionTimerDormant("b1", true);
        expect(questionTimer("b1")).toEqual({ kind: "paused", reason: "dormant" });
        noteQuestionActivity("b1"); // ignored while the tab is hidden
        vi.advanceTimersByTime(120_000);
        expect(onExpire).not.toHaveBeenCalled();

        setQuestionTimerDormant("b1", false);
        expect(questionTimer("b1")).toEqual({ kind: "counting", endsAt: Date.now() + 30_000 });
        vi.advanceTimersByTime(30_000);
        expect(onExpire).toHaveBeenCalledTimes(1);
    });

    it("starting dormant stays paused until woken", () => {
        const onExpire = vi.fn();
        startQuestionTimer("b1", { durationMs: 30_000, onExpire, dormant: true });
        expect(questionTimer("b1")).toEqual({ kind: "paused", reason: "dormant" });
        vi.advanceTimersByTime(60_000);
        expect(onExpire).not.toHaveBeenCalled();
    });

    it("end clears the state and never fires", () => {
        const onExpire = vi.fn();
        startQuestionTimer("b1", { durationMs: 30_000, onExpire });
        endQuestionTimer("b1");
        expect(questionTimer("b1")).toBeNull();
        vi.advanceTimersByTime(60_000);
        expect(onExpire).not.toHaveBeenCalled();
    });
});

describe("question timer: publishing", () => {
    it("writes block meta on edges only: start, pause, resume, end; never per tick or on repeated activity", () => {
        startQuestionTimer("b1", { durationMs: 30_000, onExpire: vi.fn(), publish: true });
        vi.advanceTimersByTime(5_000);
        expect(published()).toEqual([{ kind: "counting", endsAt: 30_000 + Date.now() - 5_000 }]);

        noteQuestionActivity("b1");
        noteQuestionActivity("b1");
        noteQuestionActivity("b1");
        expect(published()).toHaveLength(2);
        expect(published()[1]).toEqual({ kind: "paused", reason: "activity" });

        vi.advanceTimersByTime(15_000);
        expect(published()[2]).toEqual({ kind: "counting", endsAt: Date.now() + 30_000 });

        endQuestionTimer("b1");
        expect(published()).toHaveLength(4);
        expect(published()[3]).toBeNull();
        expect(writes.every((w) => w.oref === "block:b1")).toBe(true);
    });

    it("clears the published key when it expires", () => {
        startQuestionTimer("b1", { durationMs: 30_000, onExpire: vi.fn(), publish: true });
        vi.advanceTimersByTime(30_000);
        expect(published().at(-1)).toBeNull();
    });

    it("writes nothing for a local key", () => {
        startQuestionTimer("local", { durationMs: 30_000, onExpire: vi.fn() });
        noteQuestionActivity("local");
        endQuestionTimer("local");
        expect(writes).toHaveLength(0);
    });

    it("a panel mounting with nothing pending clears a key a crashed owner left behind", () => {
        setMetaByBlock({ b1: { [META_QUESTION_TIMER]: { kind: "paused", reason: "activity" } } });
        endQuestionTimer("b1", { publish: true });
        expect(published()).toEqual([null]);
    });

    it("and writes nothing when there is no key to clear", () => {
        endQuestionTimer("b1", { publish: true });
        expect(writes).toHaveLength(0);
    });
});

describe("question timer: readers", () => {
    it("a renderer that doesn't own the timer reads the published copy and schedules nothing", () => {
        const endsAt = Date.now() + 23_000;
        setMetaByBlock({ b2: { [META_QUESTION_TIMER]: { kind: "counting", endsAt } } });
        expect(questionTimer("b2")).toEqual({ kind: "counting", endsAt });
        expect(questionCountdown("b2")).toEqual({ seconds: 23, band: "default", paused: false });
        expect(vi.getTimerCount()).toBe(1); // the shared clock only, no expiry
    });

    it("ignores a published counting state whose endsAt has passed (owner crashed)", () => {
        setMetaByBlock({ b2: { [META_QUESTION_TIMER]: { kind: "counting", endsAt: Date.now() - 1_000 } } });
        expect(questionCountdown("b2")).toBeNull();
    });

    it("ignores a malformed published value", () => {
        setMetaByBlock({ b2: { [META_QUESTION_TIMER]: { kind: "counting" } } });
        expect(questionTimer("b2")).toBeNull();
    });

    it("the owner's local state wins over its own published copy", () => {
        setMetaByBlock({ b1: { [META_QUESTION_TIMER]: { kind: "paused", reason: "dormant" } } });
        startQuestionTimer("b1", { durationMs: 30_000, onExpire: vi.fn() });
        expect(questionTimer("b1")?.kind).toBe("counting");
    });

    it("counts down once per second on the shared clock", () => {
        startQuestionTimer("b1", { durationMs: 30_000, onExpire: vi.fn() });
        expect(questionCountdown("b1")?.seconds).toBe(30);
        vi.advanceTimersByTime(1_000);
        expect(questionCountdown("b1")?.seconds).toBe(29);
        vi.advanceTimersByTime(19_000);
        expect(questionCountdown("b1")).toEqual({ seconds: 10, band: "warning", paused: false });
        vi.advanceTimersByTime(5_000);
        expect(questionCountdown("b1")).toEqual({ seconds: 5, band: "critical", paused: false });
    });

    it("reads paused while paused", () => {
        startQuestionTimer("b1", { durationMs: 30_000, onExpire: vi.fn() });
        noteQuestionActivity("b1");
        expect(questionCountdown("b1")).toEqual({ seconds: 0, band: "default", paused: true });
    });

    it("bands match the panel's: ≤10s warning, ≤5s critical", () => {
        expect(countdownBand(11)).toBe("default");
        expect(countdownBand(10)).toBe("warning");
        expect(countdownBand(6)).toBe("warning");
        expect(countdownBand(5)).toBe("critical");
    });
});
