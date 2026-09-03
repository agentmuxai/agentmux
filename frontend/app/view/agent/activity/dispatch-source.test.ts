// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentDispatch } from "../../swarm/swarm-model";
import * as wos from "@/app/store/wos";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
}));

// Mirrors subagent-source.test.ts's mocking pattern exactly — see that
// file's own comment for why only `wps` is mocked, not the whole `wos`
// module.
vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));

const callBackendServiceSpy = vi.spyOn(wos, "callBackendService").mockResolvedValue([]);

import { allDispatchesAtom, msUntilNextQuietWindowRefresh, refreshDispatchesNow } from "./dispatch-source";

function mkDispatch(overrides: Partial<AgentDispatch> & Pick<AgentDispatch, "dispatch_id">): AgentDispatch {
    return {
        kind: "solo",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        member_count: 1,
        members_done: 0,
        status: "running",
        last_event_at: 0,
        dispatch_name: null,
        ...overrides,
    } as AgentDispatch;
}

describe("msUntilNextQuietWindowRefresh", () => {
    it("returns null when nothing is counts-complete", () => {
        const d = mkDispatch({ dispatch_id: "d1", member_count: 2, members_done: 1, status: "running" });
        expect(msUntilNextQuietWindowRefresh([d], 0)).toBeNull();
    });

    it("returns null for a dispatch already marked completed (not still 'running')", () => {
        // "abandoned" is a separate, not-yet-merged PR's addition to
        // AgentDispatch.status (SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md)
        // — irrelevant to this predicate either way, since it only ever
        // triggers on status === "running".
        const completed = mkDispatch({ dispatch_id: "d1", member_count: 1, members_done: 1, status: "completed" });
        expect(msUntilNextQuietWindowRefresh([completed], 0)).toBeNull();
    });

    it("returns the ms remaining until the 60s quiet window elapses for a counts-complete-but-still-running dispatch", () => {
        const d = mkDispatch({ dispatch_id: "d1", member_count: 1, members_done: 1, status: "running", last_event_at: 1000 });
        expect(msUntilNextQuietWindowRefresh([d], 1000 + 40_000)).toBe(20_000);
    });

    it("floors at 0 once the deadline has already passed", () => {
        const d = mkDispatch({ dispatch_id: "d1", member_count: 1, members_done: 1, status: "running", last_event_at: 1000 });
        expect(msUntilNextQuietWindowRefresh([d], 1000 + 90_000)).toBe(0);
    });

    it("picks the EARLIEST deadline among multiple pending dispatches", () => {
        const soon = mkDispatch({ dispatch_id: "d1", member_count: 1, members_done: 1, status: "running", last_event_at: 50_000 });
        const later = mkDispatch({ dispatch_id: "d2", member_count: 1, members_done: 1, status: "running", last_event_at: 10_000 });
        // soon's deadline: 50_000 + 60_000 = 110_000; later's: 10_000 + 60_000 = 70_000 — later is earlier in absolute terms.
        expect(msUntilNextQuietWindowRefresh([soon, later], 0)).toBe(70_000);
    });

    it("ignores a dispatch with member_count 0 (nothing to be complete about)", () => {
        const d = mkDispatch({ dispatch_id: "d1", member_count: 0, members_done: 0, status: "running" });
        expect(msUntilNextQuietWindowRefresh([d], 0)).toBeNull();
    });
});

// SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md /
// docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md: a
// backfill-replay burst on pane reopen used to fire one uncoalesced
// ListDispatches call per subagent:spawned/completed/named/abandoned or
// dispatch:updated event (up to ~200 in a real trace). These events must
// now collapse into a single call once the burst goes quiet. Mirrors
// subagent-source.test.ts's own coalescing tests for its sibling singleton.
// docs/retro/retro-activitydock-appears-on-agent-pane-load-2026-09-02.md —
// mirrors subagent-source.test.ts's identical coverage for the sibling
// singleton.
describe("dispatch-source — refreshDispatchesNow", () => {
    async function flushMicrotasks(): Promise<void> {
        await Promise.resolve();
        await Promise.resolve();
    }

    it("bypasses the debounce entirely and resolves once ListDispatches has actually landed", async () => {
        await flushMicrotasks();
        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValueOnce([mkDispatch({ dispatch_id: "d1" })]);

        await refreshDispatchesNow();

        expect(callBackendServiceSpy).toHaveBeenCalledTimes(1);
        expect(callBackendServiceSpy).toHaveBeenCalledWith("subagent", "ListDispatches", []);
        expect(allDispatchesAtom().find((d) => d.dispatch_id === "d1")).toBeDefined();
    });
});

describe("dispatch-source — refresh coalescing", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    async function flushMicrotasks(): Promise<void> {
        await Promise.resolve();
        await Promise.resolve();
    }

    it("registered handlers for every event this module is documented to refresh on", () => {
        for (const type of ["subagent:spawned", "subagent:completed", "subagent:named", "subagent:abandoned", "dispatch:updated"]) {
            expect(hub.handlers.has(type)).toBe(true);
        }
    });

    it("collapses a rapid burst of subagent:spawned events into a single ListDispatches call", async () => {
        await flushMicrotasks(); // let the module-load-time refresh() settle
        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValue([]);

        const spawned = hub.handlers.get("subagent:spawned")!;
        for (let i = 0; i < 50; i++) spawned({ data: {} });
        await flushMicrotasks();
        expect(callBackendServiceSpy).not.toHaveBeenCalled();

        await vi.advanceTimersByTimeAsync(150); // past the debounce window
        expect(callBackendServiceSpy).toHaveBeenCalledWith("subagent", "ListDispatches", []);
        expect(callBackendServiceSpy).toHaveBeenCalledTimes(1);
    });

    it("collapses a MIXED burst across different event types into a single call", async () => {
        await flushMicrotasks();
        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValue([]);

        hub.handlers.get("subagent:spawned")!({ data: {} });
        hub.handlers.get("subagent:completed")!({ data: {} });
        hub.handlers.get("dispatch:updated")!({ data: {} });
        hub.handlers.get("subagent:abandoned")!({ data: {} });
        await flushMicrotasks();
        expect(callBackendServiceSpy).not.toHaveBeenCalled();

        await vi.advanceTimersByTimeAsync(150);
        expect(callBackendServiceSpy).toHaveBeenCalledTimes(1);
    });
});

// docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md:
// the debounce above coalesces request VOLUME, but each surviving call
// during a burst is still a genuinely different, real, still-converging
// snapshot — rows still visibly appear/vanish. backfill-tracker.ts closes
// this by suppressing refresh entirely while ANY block's backfill is
// reported in flight, firing exactly one once it's genuinely done.
describe("dispatch-source — backfill-aware suppression", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    async function flushMicrotasks(): Promise<void> {
        await Promise.resolve();
        await Promise.resolve();
    }

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
