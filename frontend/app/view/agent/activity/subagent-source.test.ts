// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * subagent-source.ts — the Activity Dock's subagent-list singleton.
 *
 * Regression test for the stale-dock-row bug: after an app restart, a
 * subagent the backend correctly reconciles from `"active"` to
 * `"abandoned"` (see `reconcile_stale_subagents`, which runs on every pane
 * reopen with a persisted session id) never made this module refresh —
 * `subagent:abandoned` wasn't in its event-subscription list, unlike its
 * sibling `dispatch-source.ts` (which got the identical fix per reagent/
 * codex, PR #2676). The dock kept showing the pre-restart snapshot
 * ("running", frozen timestamp/event count) indefinitely.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ActiveSubagent } from "../../swarm/swarm-model";
import * as wos from "@/app/store/wos";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
}));

// Only `wps` is mocked (to capture handlers synchronously instead of going
// through the real WAVE event bus) — `wos` is NOT module-mocked, mirroring
// dispatch-source.test.ts: other code reachable from this import graph
// (window-identity.ts's `tabAtom`) needs wos's other exports (getObjectValue/
// makeORef) to exist for real. `callBackendService` itself is spied on
// below instead of the whole module being replaced.
vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));

const callBackendServiceSpy = vi.spyOn(wos, "callBackendService").mockResolvedValue([]);

import { allSubagentsAtom, refreshSubagentsNow } from "./subagent-source";

function mkSubagent(overrides: Partial<ActiveSubagent> & Pick<ActiveSubagent, "agent_id">): ActiveSubagent {
    return {
        slug: "s1",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        status: "active",
        spawned_at: 0,
        last_event_at: 0,
        event_count: 1,
        model: null,
        ...overrides,
    } as ActiveSubagent;
}

// The module under test wires its subscriptions and fires its first
// `refresh()` at IMPORT time (a bare module-level singleton, not a hook) —
// every test in this file shares that one singleton instance/subscription
// set (module-load side effects only ever run once per test file).

// Let the module's own module-load-time `void refresh()` promise settle
// before each assertion block starts counting calls from a clean baseline.
async function flushMicrotasks(): Promise<void> {
    await Promise.resolve();
    await Promise.resolve();
}

// Fake timers for the whole file: every event-triggered refresh now goes
// through createDebouncedRefresh (SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md),
// so tests need to advance past its wait window. Fake timers don't affect
// Promise microtask resolution, so flushMicrotasks() above still works
// unchanged alongside them.
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

// docs/retro/retro-activitydock-appears-on-agent-pane-load-2026-09-02.md:
// `useSubagentBackfillGate` needs to know when the dock's OWN data has
// actually caught up with a settled backfill, not guess a fixed duration.
describe("subagent-source — refreshSubagentsNow", () => {
    it("bypasses the debounce entirely and resolves once ListActive has actually landed", async () => {
        await flushMicrotasks();
        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValueOnce([mkSubagent({ agent_id: "a1" })]);

        await refreshSubagentsNow();

        expect(callBackendServiceSpy).toHaveBeenCalledTimes(1);
        expect(callBackendServiceSpy).toHaveBeenCalledWith("subagent", "ListActive", []);
        expect(allSubagentsAtom().find((s) => s.agent_id === "a1")).toBeDefined();
    });
});

describe("subagent-source — subagent:abandoned wiring", () => {
    it("registered a handler for subagent:abandoned (the fix's own precondition)", () => {
        expect(hub.handlers.has("subagent:abandoned")).toBe(true);
    });

    it("refreshes (re-fetches ListActive) when subagent:abandoned fires", async () => {
        await flushMicrotasks(); // let the module-load-time refresh() settle
        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValueOnce([
            mkSubagent({ agent_id: "a1", status: "abandoned" }),
        ]);

        const handler = hub.handlers.get("subagent:abandoned");
        expect(handler).toBeDefined();
        handler!({ data: {} });
        await vi.advanceTimersByTimeAsync(150); // past the debounce window
        await flushMicrotasks();

        expect(callBackendServiceSpy).toHaveBeenCalledWith("subagent", "ListActive", []);
        expect(allSubagentsAtom().find((s) => s.agent_id === "a1")?.status).toBe("abandoned");
    });

    it("also still refreshes on subagent:spawned and subagent:completed (unchanged behavior)", async () => {
        await flushMicrotasks();
        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValue([]);

        hub.handlers.get("subagent:spawned")!({ data: {} });
        await vi.advanceTimersByTimeAsync(150);
        await flushMicrotasks();
        expect(callBackendServiceSpy).toHaveBeenCalledTimes(1);

        hub.handlers.get("subagent:completed")!({ data: {} });
        await vi.advanceTimersByTimeAsync(150);
        await flushMicrotasks();
        expect(callBackendServiceSpy).toHaveBeenCalledTimes(2);
    });
});

// SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md /
// docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md: a
// backfill-replay burst on pane reopen used to fire one uncoalesced
// ListActive call per event (up to ~200 in a real trace). These events must
// now collapse into a single call once the burst goes quiet.
describe("subagent-source — refresh coalescing", () => {
    it("collapses a rapid burst of subagent:spawned events into a single ListActive call", async () => {
        await flushMicrotasks(); // let the module-load-time refresh() settle
        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValue([]);

        const spawned = hub.handlers.get("subagent:spawned")!;
        for (let i = 0; i < 50; i++) spawned({ data: {} });
        await flushMicrotasks();
        expect(callBackendServiceSpy).not.toHaveBeenCalled();

        await vi.advanceTimersByTimeAsync(150); // past the debounce window
        expect(callBackendServiceSpy).toHaveBeenCalledTimes(1);
    });

    it("still refreshes under a SUSTAINED burst, via the max-wait ceiling, without waiting for it to fully quiet", async () => {
        await flushMicrotasks();
        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValue([]);

        const spawned = hub.handlers.get("subagent:spawned")!;
        // Re-fire every 60ms (under the 100ms debounce window) for longer
        // than the 1000ms ceiling — mirrors the real trace's ~14ms average
        // spacing across a multi-second backfill burst.
        for (let i = 0; i < 20; i++) {
            spawned({ data: {} });
            await vi.advanceTimersByTimeAsync(60);
        }
        await flushMicrotasks();
        expect(callBackendServiceSpy.mock.calls.length).toBeGreaterThanOrEqual(1);
    });
});

// docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md:
// the debounce above coalesces request VOLUME, but each surviving call
// during a burst is still a genuinely different, real, still-converging
// snapshot — rows still visibly appear/vanish. backfill-tracker.ts closes
// this by suppressing refresh entirely while ANY block's backfill is
// reported in flight, firing exactly one once it's genuinely done. Mirrors
// dispatch-source.test.ts's identical coverage for the sibling singleton.
describe("subagent-source — backfill-aware suppression", () => {
    it("suppresses refresh entirely for events arriving while a backfill is in flight, firing exactly one once it settles", async () => {
        await flushMicrotasks();
        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValue([]);

        const backfillStatus = hub.handlers.get("subagent:backfill_status")!;
        expect(backfillStatus).toBeDefined();
        backfillStatus({ scopes: ["block:b1"], data: { status: "started" } });

        const spawned = hub.handlers.get("subagent:spawned")!;
        for (let i = 0; i < 50; i++) spawned({ data: {} });
        await vi.advanceTimersByTimeAsync(1200); // well past both debounce windows
        expect(callBackendServiceSpy).not.toHaveBeenCalled(); // suppressed, not just debounced

        backfillStatus({ scopes: ["block:b1"], data: { status: "done" } });
        await flushMicrotasks();
        expect(callBackendServiceSpy).toHaveBeenCalledTimes(1); // exactly one, on settle
    });

    it("resumes ordinary debounced behavior for events after the backfill settles", async () => {
        await flushMicrotasks();
        const backfillStatus = hub.handlers.get("subagent:backfill_status")!;
        backfillStatus({ scopes: ["block:b2"], data: { status: "started" } });
        backfillStatus({ scopes: ["block:b2"], data: { status: "done" } });
        await flushMicrotasks();

        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValue([]);
        hub.handlers.get("subagent:spawned")!({ data: {} });
        await vi.advanceTimersByTimeAsync(150);
        expect(callBackendServiceSpy).toHaveBeenCalledTimes(1);
    });
});
