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

import { describe, expect, it, vi } from "vitest";
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

import { allSubagentsAtom } from "./subagent-source";

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
        await flushMicrotasks();

        expect(callBackendServiceSpy).toHaveBeenCalledWith("subagent", "ListActive", []);
        expect(allSubagentsAtom().find((s) => s.agent_id === "a1")?.status).toBe("abandoned");
    });

    it("also still refreshes on subagent:spawned and subagent:completed (unchanged behavior)", async () => {
        await flushMicrotasks();
        callBackendServiceSpy.mockClear();
        callBackendServiceSpy.mockResolvedValue([]);

        hub.handlers.get("subagent:spawned")!({ data: {} });
        await flushMicrotasks();
        expect(callBackendServiceSpy).toHaveBeenCalledTimes(1);

        hub.handlers.get("subagent:completed")!({ data: {} });
        await flushMicrotasks();
        expect(callBackendServiceSpy).toHaveBeenCalledTimes(2);
    });
});
