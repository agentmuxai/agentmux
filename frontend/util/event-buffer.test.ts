// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.6 — tests for the per-source tracker.
//
// Covers: version monotonicity, gap detection, stale drop, saga
// buffer round-trip, nested-saga flush, terminal-without-start
// pass-through, malformed-event rejection, subscriber error
// isolation, overflow safeguard.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { PerSourceTracker, type VersionedEvent } from "./event-buffer";

interface TestEvent extends VersionedEvent {}

interface MockSetters {
    setLatest: ReturnType<typeof vi.fn>;
    setVersion: ReturnType<typeof vi.fn>;
    setSawAny: ReturnType<typeof vi.fn>;
}

function mockSetters(): MockSetters {
    return {
        setLatest: vi.fn(),
        setVersion: vi.fn(),
        setSawAny: vi.fn(),
    };
}

function evt(name: string, version: number, extra: Record<string, unknown> = {}): TestEvent {
    return { event: name, version, ...extra };
}

describe("PerSourceTracker — basic delivery", () => {
    let setters: MockSetters;
    let tracker: PerSourceTracker<TestEvent>;
    let onGap: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        setters = mockSetters();
        onGap = vi.fn();
        tracker = new PerSourceTracker<TestEvent>(
            { source: "test", onVersionGap: onGap },
            setters,
        );
    });

    it("dispatches first event and flips sawAny", () => {
        tracker.deliver(evt("workspace_created", 1));
        expect(setters.setLatest).toHaveBeenCalledTimes(1);
        expect(setters.setVersion).toHaveBeenCalledWith(1);
        expect(setters.setSawAny).toHaveBeenCalledWith(true);
    });

    it("dispatches subsequent events in order with monotonic versions", () => {
        tracker.deliver(evt("a", 1));
        tracker.deliver(evt("b", 2));
        tracker.deliver(evt("c", 3));
        expect(setters.setLatest).toHaveBeenCalledTimes(3);
        expect(setters.setVersion).toHaveBeenLastCalledWith(3);
    });

    it("invokes subscribers per event in order", () => {
        const seen: string[] = [];
        tracker.subscribe((e) => seen.push(`${e.event}@${e.version}`));
        tracker.deliver(evt("a", 1));
        tracker.deliver(evt("b", 2));
        expect(seen).toEqual(["a@1", "b@2"]);
    });

    it("returns an unsubscribe function", () => {
        const cb = vi.fn();
        const unsub = tracker.subscribe(cb);
        tracker.deliver(evt("a", 1));
        unsub();
        tracker.deliver(evt("b", 2));
        expect(cb).toHaveBeenCalledTimes(1);
    });
});

describe("PerSourceTracker — version checking", () => {
    let setters: MockSetters;
    let tracker: PerSourceTracker<TestEvent>;
    let onGap: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        setters = mockSetters();
        onGap = vi.fn();
        tracker = new PerSourceTracker<TestEvent>(
            { source: "test", onVersionGap: onGap },
            setters,
        );
        // Silence the default console.warn for stale events. Other
        // tests installed a custom onVersionGap so they don't need
        // this; this stub covers the stale path.
        vi.spyOn(console, "warn").mockImplementation(() => {});
    });

    it("logs a gap when version skips ahead", () => {
        tracker.deliver(evt("a", 1));
        tracker.deliver(evt("b", 5)); // gap of 3
        expect(onGap).toHaveBeenCalledWith(3, 1, 5);
        expect(tracker.stats().droppedCount).toBe(3);
    });

    it("does NOT log a gap on the very first event regardless of version", () => {
        tracker.deliver(evt("a", 100));
        expect(onGap).not.toHaveBeenCalled();
        expect(tracker.stats().droppedCount).toBe(0);
        expect(tracker.stats().lastVersion).toBe(100);
    });

    it("drops stale events (version <= last seen)", () => {
        tracker.deliver(evt("a", 5));
        tracker.deliver(evt("b", 5));
        tracker.deliver(evt("c", 3));
        expect(setters.setLatest).toHaveBeenCalledTimes(1);
        expect(tracker.stats().lastVersion).toBe(5);
    });

    it("resets state when source restarts (codex P1: version=1 after lastVersion>0)", () => {
        // Regression: srv/launcher reset event_version=0 on process
        // restart. After restart, version=1 events were being dropped
        // as stale (1 <= lastVersion=42), permanently black-holing
        // the stream until page reload.
        tracker.deliver(evt("a", 40));
        tracker.deliver(evt("b", 41));
        tracker.deliver(evt("c", 42));
        expect(tracker.stats().lastVersion).toBe(42);

        // Source restart — first post-restart event is version=1.
        tracker.deliver(evt("post_restart_first", 1));
        expect(setters.setLatest).toHaveBeenCalledTimes(4);
        expect(tracker.stats().lastVersion).toBe(1);
        expect(tracker.stats().droppedCount).toBe(0); // reset by restart

        // Subsequent events after restart should flow normally.
        tracker.deliver(evt("post_restart_second", 2));
        expect(setters.setLatest).toHaveBeenCalledTimes(5);
        expect(tracker.stats().lastVersion).toBe(2);
    });

    it("drops stale saga buffer on source restart", () => {
        // If we were buffering a saga when the source restarted, the
        // buffer is part of the dead source's history. Drop it.
        tracker.deliver(evt("a", 40));
        tracker.deliver(evt("saga_started", 41, { saga_id: 7 }));
        tracker.deliver(evt("step", 42));
        expect(tracker.stats().inSaga).toBe(7);
        expect(tracker.stats().bufferedCount).toBe(2);

        // Source restart.
        tracker.deliver(evt("fresh_event", 1));
        expect(tracker.stats().inSaga).toBeNull();
        expect(tracker.stats().bufferedCount).toBe(0);
        expect(tracker.stats().lastVersion).toBe(1);
    });
});

describe("PerSourceTracker — saga buffering", () => {
    let setters: MockSetters;
    let tracker: PerSourceTracker<TestEvent>;
    let onTerminal: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        setters = mockSetters();
        onTerminal = vi.fn();
        tracker = new PerSourceTracker<TestEvent>(
            { source: "test", onSagaTerminal: onTerminal },
            setters,
        );
    });

    it("buffers events between saga_started and saga_completed", () => {
        const seen: string[] = [];
        tracker.subscribe((e) => seen.push(`${e.event}@${e.version}`));
        tracker.deliver(evt("saga_started", 1, { saga_id: 42, name: "tear_off_tab" }));
        // Subscriber should NOT have been called yet (buffered).
        expect(seen).toEqual([]);
        tracker.deliver(evt("tab_moved", 2));
        tracker.deliver(evt("block_moved", 3));
        expect(seen).toEqual([]);
        tracker.deliver(evt("saga_completed", 4, { saga_id: 42 }));
        // All 4 events delivered in order, in one batch.
        expect(seen).toEqual(["saga_started@1", "tab_moved@2", "block_moved@3", "saga_completed@4"]);
        expect(onTerminal).toHaveBeenCalledWith(42, "completed");
    });

    it("buffers events on saga_failed and reports failed terminal", () => {
        const seen: string[] = [];
        tracker.subscribe((e) => seen.push(e.event));
        tracker.deliver(evt("saga_started", 1, { saga_id: 7 }));
        tracker.deliver(evt("tab_moved", 2));
        tracker.deliver(evt("saga_failed", 3, { saga_id: 7, reason: "boom" }));
        expect(seen).toEqual(["saga_started", "tab_moved", "saga_failed"]);
        expect(onTerminal).toHaveBeenCalledWith(7, "failed");
    });

    it("flushes prior saga and opens new one on nested saga_started", () => {
        vi.spyOn(console, "warn").mockImplementation(() => {});
        const seen: string[] = [];
        tracker.subscribe((e) => seen.push(`${e.event}@${e.saga_id ?? ""}`));
        tracker.deliver(evt("saga_started", 1, { saga_id: 10 }));
        tracker.deliver(evt("step_a", 2));
        tracker.deliver(evt("saga_started", 3, { saga_id: 11 })); // nested → flush prior
        // Prior saga 10 + step_a flushed (no terminal — partial)
        expect(seen).toContain("saga_started@10");
        expect(seen).toContain("step_a@");
        // saga 11 NOT yet delivered (still buffering)
        expect(seen).not.toContain("saga_started@11");
        tracker.deliver(evt("saga_completed", 4, { saga_id: 11 }));
        expect(seen).toContain("saga_started@11");
        expect(seen).toContain("saga_completed@11");
    });

    it("passes through terminal that does not match an in-flight saga", () => {
        const seen: string[] = [];
        tracker.subscribe((e) => seen.push(e.event));
        tracker.deliver(evt("saga_completed", 1, { saga_id: 99 })); // no matching start
        expect(seen).toEqual(["saga_completed"]);
        expect(onTerminal).not.toHaveBeenCalled();
    });

    it("flushes prior buffer before mismatched terminal (reagent P1 + codex P1 round 2)", () => {
        // Regression — both P1s on PR #630:
        //   1. Mismatched terminal must NOT be buried inside the
        //      unrelated in-flight saga's buffer (reagent).
        //   2. AND it must NOT be dispatched before the buffered
        //      events (codex round 2: violates source ordering /
        //      monotonic version contract).
        // Resolution: flush prior buffer first, then dispatch
        // mismatched terminal. Order preserved, nothing buried.
        vi.spyOn(console, "warn").mockImplementation(() => {});
        const seen: { event: string; saga_id: unknown; version: number }[] = [];
        tracker.subscribe((e) =>
            seen.push({ event: e.event, saga_id: e.saga_id, version: e.version }),
        );
        tracker.deliver(evt("saga_started", 1, { saga_id: 99 }));
        tracker.deliver(evt("step_a", 2)); // buffered
        // Mismatched terminal at version 3 arrives mid-99
        tracker.deliver(evt("saga_completed", 3, { saga_id: 88 }));

        // Prior buffer flushed in source order, then mismatched
        // terminal delivered last.
        expect(seen.map((e) => e.event)).toEqual(["saga_started", "step_a", "saga_completed"]);
        expect(seen.map((e) => e.version)).toEqual([1, 2, 3]); // monotonic
        expect(seen[0].saga_id).toBe(99);
        expect(seen[2].saga_id).toBe(88);
        // Saga 99's buffer drained; tracker idle.
        expect(tracker.stats().inSaga).toBeNull();
    });

    it("emergency-flushes on overflow", () => {
        vi.spyOn(console, "warn").mockImplementation(() => {});
        const flushSize = 5;
        const t = new PerSourceTracker<TestEvent>(
            { source: "test", maxSagaBufferSize: flushSize },
            setters,
        );
        const seen: string[] = [];
        t.subscribe((e) => seen.push(e.event));
        t.deliver(evt("saga_started", 1, { saga_id: 1 }));
        for (let i = 2; i <= 10; i++) {
            t.deliver(evt(`step_${i}`, i));
        }
        // After 6 events past saga_started (1 saga_started + 9 steps),
        // overflow at the 7th queued (events length > 5) triggers flush.
        // Subsequent steps deliver immediately.
        expect(seen.length).toBeGreaterThan(0);
        expect(t.stats().inSaga).toBeNull();
    });
});

describe("PerSourceTracker — defensive paths", () => {
    let setters: MockSetters;
    let tracker: PerSourceTracker<TestEvent>;

    beforeEach(() => {
        setters = mockSetters();
        tracker = new PerSourceTracker<TestEvent>(
            { source: "test" },
            setters,
        );
        vi.spyOn(console, "warn").mockImplementation(() => {});
        vi.spyOn(console, "error").mockImplementation(() => {});
    });

    it("rejects events missing required fields", () => {
        // @ts-expect-error - intentional malformed event for test
        tracker.deliver({ event: "a" });
        // @ts-expect-error - intentional malformed event for test
        tracker.deliver({ version: 1 });
        tracker.deliver(null as unknown as TestEvent);
        expect(setters.setLatest).not.toHaveBeenCalled();
    });

    it("isolates subscriber exceptions from later subscribers", () => {
        const calls: string[] = [];
        tracker.subscribe(() => {
            throw new Error("subscriber A blew up");
        });
        tracker.subscribe((e) => calls.push(e.event));
        tracker.deliver(evt("a", 1));
        expect(calls).toEqual(["a"]);
    });

    it("treats saga_started without saga_id as a plain event", () => {
        const seen: string[] = [];
        tracker.subscribe((e) => seen.push(e.event));
        tracker.deliver(evt("saga_started", 1)); // no saga_id
        expect(seen).toEqual(["saga_started"]);
        expect(tracker.stats().inSaga).toBeNull();
    });

    it("stats reflects ongoing saga state", () => {
        tracker.deliver(evt("saga_started", 1, { saga_id: 5 }));
        tracker.deliver(evt("step", 2));
        const s = tracker.stats();
        expect(s.inSaga).toBe(5);
        expect(s.bufferedCount).toBe(2); // saga_started + step
    });
});
