// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for the tab-content reveal gate (issue #774).
// Specifically guards the holdRevealGate / scheduleRevealLift pairing:
// during async work without longtasks (RPCs, layout-model polling),
// the SETTLE window must NOT elapse and prematurely reveal the tab.

import { describe, test, expect, beforeEach, afterEach, vi } from "vitest";
import { createRoot } from "solid-js";
import {
    clearLeafRevealGate,
    gatingNodeIds,
    holdLeafRevealGate,
    holdRevealGate,
    scheduleLeafRevealLift,
    scheduleRevealLift,
    tabSwitching,
} from "./tab-reveal";

function read<T>(signal: () => T): T {
    let val!: T;
    createRoot((dispose) => {
        val = signal();
        dispose();
    });
    return val;
}

// All PerformanceObserver instances installed by scheduleRevealLift land
// here so tests can drive the rAF tick path without a real browser.
// `requestAnimationFrame` is stubbed by vi.useFakeTimers below.

describe("tab-reveal gate", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        // Force the no-PerformanceObserver fallback path so the gate
        // lift behaviour is entirely timer-driven and deterministic.
        // The real production path uses PerformanceObserver longtask
        // entries; the SETTLE-elapsed-during-await bug this test
        // guards reproduces identically in the fallback path because
        // both rely on SETTLE_MS elapsing without "busy" signal.
        // Strip PerformanceObserver for the fallback path (it is not
        // strictly typed on globalThis under our tsconfig, so a plain
        // assignment is fine — no ts-expect-error needed).
        (globalThis as { PerformanceObserver?: unknown }).PerformanceObserver = undefined;
    });

    afterEach(() => {
        // Reset so subsequent tests start with the gate down.
        // scheduleRevealLift + fast-forward triggers the fallback
        // timer (MAX_GATE_MS=800) to clear the signal; advance past
        // it to drain.
        scheduleRevealLift();
        vi.advanceTimersByTime(1000);
        vi.useRealTimers();
    });

    test("holdRevealGate raises the gate", () => {
        expect(read(tabSwitching)).toBe(false);
        holdRevealGate();
        expect(read(tabSwitching)).toBe(true);
    });

    test("holdRevealGate keeps the gate up across long awaits", () => {
        // Simulates the createTab / setActiveTab failure mode: an RPC
        // that runs longer than SETTLE_MS (80ms) with no longtasks
        // firing. Under the old code (scheduleRevealLift before await),
        // the fallback timer would lift the gate after 80ms even
        // though the destination tab had not yet mounted.
        holdRevealGate();
        vi.advanceTimersByTime(500); // way past SETTLE_MS, still under MAX_GATE_MS
        expect(read(tabSwitching)).toBe(true);
    });

    test("holdRevealGate safety-lifts after MAX_GATE_MS if no schedule follows", () => {
        // Codex P2: if the awaited RPC never settles (callBackendService
        // has no timeout), the paired scheduleRevealLift in `finally`
        // never runs and the gate would stay up forever, leaving the
        // window blank indefinitely. The safety net inside
        // holdRevealGate prevents this — gate auto-lifts at the hard
        // cap even with no paired schedule.
        holdRevealGate();
        vi.advanceTimersByTime(900); // past MAX_GATE_MS=800
        expect(read(tabSwitching)).toBe(false);
    });

    test("scheduleRevealLift after holdRevealGate eventually lifts via fallback", () => {
        holdRevealGate();
        vi.advanceTimersByTime(500);
        expect(read(tabSwitching)).toBe(true);
        // Pair the hold with a schedule once the simulated async work
        // completes. The fallback timer arms now (MAX_GATE_MS=800ms)
        // and fires, dropping the gate.
        scheduleRevealLift();
        // Still well within the hard cap — gate stays up.
        vi.advanceTimersByTime(400);
        expect(read(tabSwitching)).toBe(true);
        // Past MAX_GATE_MS — fallback fires, gate drops.
        vi.advanceTimersByTime(500);
        expect(read(tabSwitching)).toBe(false);
    });

    test("holdRevealGate cancels a pending fallback timer from a prior schedule", () => {
        // The same stale-fallback-timer class of bug that PR commit
        // 986c92ba fixed for re-entry of scheduleRevealLift — verify
        // holdRevealGate also cancels it. Otherwise a hold issued
        // shortly after a schedule could see the stale timer fire and
        // drop the gate mid-await.
        scheduleRevealLift();
        // Get well into the original fallback window, then re-enter
        // via holdRevealGate.
        vi.advanceTimersByTime(400);
        holdRevealGate();
        // Past where the prior schedule's MAX_GATE_MS would have fired
        // (800ms from the schedule call, i.e. 400ms after the hold).
        // Stay under the hold's OWN safety net so we're verifying the
        // prior timer's cancellation, not gate-still-up by other means.
        vi.advanceTimersByTime(500);
        expect(read(tabSwitching)).toBe(true);
    });

    test("rapid hold→schedule→hold sequence keeps the gate up", () => {
        // setActiveTab spam case: each call holds, awaits, then
        // schedules. A subsequent call must re-hold before the prior
        // call's schedule fallback timer fires.
        holdRevealGate();
        vi.advanceTimersByTime(30);
        scheduleRevealLift();
        // Well within MAX_GATE_MS, so the gate is still up.
        vi.advanceTimersByTime(400);
        expect(read(tabSwitching)).toBe(true);
        holdRevealGate();
        // Past where the prior schedule's fallback would have fired,
        // but under the hold's own safety net.
        vi.advanceTimersByTime(500);
        expect(read(tabSwitching)).toBe(true);
    });
});

// Leaf-scoped gate (SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22) — same
// detector primitive as the whole-tab gate above, but keyed per layout
// node id instead of one global boolean, so more than one pane can be
// independently settling at once (e.g. two block-stack pushes in
// different panes of the same tab). Generation-token API: every
// `holdLeafRevealGate` call returns a token that must be threaded through
// to the paired `scheduleLeafRevealLift` call — see tab-reveal.ts's own
// module doc comment for the two races (Codex's review of PR #2761) this
// exists to close.
describe("tab-reveal leaf-scoped gate", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        (globalThis as { PerformanceObserver?: unknown }).PerformanceObserver = undefined;
    });

    afterEach(() => {
        // Drain any leaf handles left gating from a test that didn't
        // schedule its own lift. Uses a fresh hold+schedule pair (rather
        // than a bare call with an arbitrary generation) so it reliably
        // owns and lifts the gate regardless of what generation the test
        // left node-a/node-b on.
        scheduleLeafRevealLift("node-a", holdLeafRevealGate("node-a"));
        scheduleLeafRevealLift("node-b", holdLeafRevealGate("node-b"));
        vi.advanceTimersByTime(1000);
        vi.useRealTimers();
    });

    test("holdLeafRevealGate adds the node id to gatingNodeIds", () => {
        expect(read(gatingNodeIds).has("node-a")).toBe(false);
        holdLeafRevealGate("node-a");
        expect(read(gatingNodeIds).has("node-a")).toBe(true);
    });

    test("holdLeafRevealGate keeps the node gated across long awaits", () => {
        holdLeafRevealGate("node-a");
        vi.advanceTimersByTime(500); // past SETTLE_MS, under MAX_GATE_MS
        expect(read(gatingNodeIds).has("node-a")).toBe(true);
    });

    test("holdLeafRevealGate safety-lifts after MAX_GATE_MS if no schedule follows", () => {
        holdLeafRevealGate("node-a");
        vi.advanceTimersByTime(900); // past MAX_GATE_MS=800
        expect(read(gatingNodeIds).has("node-a")).toBe(false);
    });

    test("scheduleLeafRevealLift after holdLeafRevealGate eventually lifts via fallback", () => {
        const gen = holdLeafRevealGate("node-a");
        vi.advanceTimersByTime(500);
        expect(read(gatingNodeIds).has("node-a")).toBe(true);
        scheduleLeafRevealLift("node-a", gen);
        vi.advanceTimersByTime(400);
        expect(read(gatingNodeIds).has("node-a")).toBe(true);
        vi.advanceTimersByTime(500);
        expect(read(gatingNodeIds).has("node-a")).toBe(false);
    });

    test("two different node ids gate and lift completely independently", () => {
        // node-a's own MAX_GATE_MS window is armed at t=0 (fires at t=800)
        // and deliberately left alone (no re-hold/schedule) so it lifts
        // via its own safety net — node-b is held/rescheduled AFTER that
        // point so its window doesn't coincide with node-a's.
        holdLeafRevealGate("node-a");
        holdLeafRevealGate("node-b");
        vi.advanceTimersByTime(700); // t=700 — neither has hit its 800ms cap yet
        expect(read(gatingNodeIds).has("node-a")).toBe(true);
        expect(read(gatingNodeIds).has("node-b")).toBe(true);

        // Push node-b's window out further without touching node-a.
        const genB = holdLeafRevealGate("node-b");
        vi.advanceTimersByTime(200); // t=900 — node-a's original 800ms cap has passed
        expect(read(gatingNodeIds).has("node-a")).toBe(false);
        // node-b was re-held at t=700, so its cap is at t=1500 — still gated.
        expect(read(gatingNodeIds).has("node-b")).toBe(true);

        // Now lift node-b.
        scheduleLeafRevealLift("node-b", genB);
        vi.advanceTimersByTime(900);
        expect(read(gatingNodeIds).has("node-b")).toBe(false);
    });

    test("holdLeafRevealGate cancels a pending fallback timer from a prior schedule on the SAME node only", () => {
        const genA1 = holdLeafRevealGate("node-a");
        scheduleLeafRevealLift("node-a", genA1);
        holdLeafRevealGate("node-b");
        vi.advanceTimersByTime(400);
        holdLeafRevealGate("node-a");
        vi.advanceTimersByTime(500);
        // node-a's re-hold cancelled its own prior schedule's fallback —
        // still gated via the new hold's own safety net.
        expect(read(gatingNodeIds).has("node-a")).toBe(true);
    });

    test("the whole-tab gate and a leaf gate are fully independent of each other", () => {
        // Both armed at t=0 (fires at t=800 if left alone). The leaf is
        // re-held at t=700 to push its own window past the point where
        // the tab gate is checked, proving one lifting doesn't touch the
        // other (rather than both coincidentally expiring together).
        holdRevealGate();
        holdLeafRevealGate("node-a");
        expect(read(tabSwitching)).toBe(true);
        expect(read(gatingNodeIds).has("node-a")).toBe(true);

        vi.advanceTimersByTime(700); // t=700 — neither has hit its cap yet
        const gen = holdLeafRevealGate("node-a"); // re-arm node-a's window to fire at t=1500

        vi.advanceTimersByTime(200); // t=900 — tab's original 800ms cap has passed
        expect(read(tabSwitching)).toBe(false);
        // The tab lifting via its own cap must not affect the leaf gate.
        expect(read(gatingNodeIds).has("node-a")).toBe(true);

        scheduleLeafRevealLift("node-a", gen);
        vi.advanceTimersByTime(900);
        expect(read(gatingNodeIds).has("node-a")).toBe(false);
    });

    // Codex's review of PR #2761, race #1: two overlapping operations on
    // the SAME node id (e.g. two rapid "+" clicks before the first's RPC
    // resolves). The OLDER operation's completion must not reveal the pane
    // while the NEWER operation is still in flight.
    test("an OLDER operation's schedule call does not reveal the pane while a NEWER operation is still in flight", () => {
        const genOld = holdLeafRevealGate("node-a"); // "click 1", cap at t=800
        vi.advanceTimersByTime(100); // t=100
        const genNew = holdLeafRevealGate("node-a"); // "click 2" supersedes click 1, cap at t=900
        expect(genNew).not.toBe(genOld);

        // Click 1's own async work "finishes" first (at t=100) and
        // schedules its lift — must be a no-op now that click 2 owns the
        // gate. If this incorrectly started its OWN settle-detector,
        // node-a would reveal ~80ms later (SETTLE_MS, with no PerformanceObserver
        // so via the no-PO MAX_GATE_MS-only fallback path this test forces —
        // still well before click 2's own 800ms safety net), which is
        // exactly the bug this generation check prevents.
        scheduleLeafRevealLift("node-a", genOld);
        vi.advanceTimersByTime(200); // t=300 — past where a stray detector would have fired, well under click 2's t=900 cap
        expect(read(gatingNodeIds).has("node-a")).toBe(true);

        // Click 2 eventually finishes and schedules its OWN lift — this
        // one is real and should actually reveal the pane once settled.
        scheduleLeafRevealLift("node-a", genNew);
        vi.advanceTimersByTime(900);
        expect(read(gatingNodeIds).has("node-a")).toBe(false);
    });

    // Codex's review of PR #2761, race #2: a single SLOW operation whose
    // own hold safety-net fires (revealing the pane) before the operation
    // actually finishes. The later schedule call must not re-hide an
    // already-revealed pane — that visible→hidden→visible flash is worse
    // than just leaving it visible while the slow operation wraps up.
    test("does not re-hide an already-revealed pane when a slow operation's hold already timed out", () => {
        const gen = holdLeafRevealGate("node-a");
        vi.advanceTimersByTime(900); // past MAX_GATE_MS=800 — hold's own safety net fires
        expect(read(gatingNodeIds).has("node-a")).toBe(false); // revealed early, as designed

        // The slow operation FINALLY finishes and calls its paired
        // schedule — this generation already resolved once, so it must
        // be a no-op, not a fresh hide-then-reveal cycle.
        scheduleLeafRevealLift("node-a", gen);
        expect(read(gatingNodeIds).has("node-a")).toBe(false);
        vi.advanceTimersByTime(200);
        expect(read(gatingNodeIds).has("node-a")).toBe(false);
    });

    // reagent's review of PR #2761: leafCancels/leafGeneration/
    // leafResolvedGeneration are otherwise only ever added to or
    // overwritten, never deleted — clearLeafRevealGate is the cleanup hook
    // wired from closeNode (layoutMagnify.ts) when a node id is gone for good.
    describe("clearLeafRevealGate", () => {
        test("removes a currently-gated node id from gatingNodeIds and cancels its pending timer", () => {
            holdLeafRevealGate("node-a");
            expect(read(gatingNodeIds).has("node-a")).toBe(true);

            clearLeafRevealGate("node-a");
            expect(read(gatingNodeIds).has("node-a")).toBe(false);

            // The cancelled hold's own MAX_GATE_MS timer must not fire and
            // re-add the node after cleanup.
            vi.advanceTimersByTime(900);
            expect(read(gatingNodeIds).has("node-a")).toBe(false);
        });

        test("a stale generation's schedule call is still a no-op after clearLeafRevealGate (no resurrection)", () => {
            const gen = holdLeafRevealGate("node-a");
            clearLeafRevealGate("node-a");

            // Same node id, fresh use afterward (e.g. a new pane created and
            // reusing a previously-cleared node id) — the OLD generation's
            // stale schedule call must not resurrect gating for it.
            scheduleLeafRevealLift("node-a", gen);
            expect(read(gatingNodeIds).has("node-a")).toBe(false);
        });

        test("is a harmless no-op for a node id that was never gated", () => {
            expect(() => clearLeafRevealGate("node-never-gated")).not.toThrow();
            expect(read(gatingNodeIds).has("node-never-gated")).toBe(false);
        });
    });
});
