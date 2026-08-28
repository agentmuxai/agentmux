// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Issue: agentmuxai/agentmux#2829 — backfilled/historical subagents never
 * got a resolved display_name, only a per-row on-click fallback that never
 * covers the default collapsed view. The fix fires a bounded backlog-naming
 * RPC once, when a human actually opens the Swarm pane — this is the one
 * moment `SwarmViewModel` is constructed (block-registry.ts's "swarm" view),
 * never from the headless per-agent-pane backfill scan.
 *
 * Mocking follows subagent-source.test.ts's pattern: only `wps` is
 * module-mocked (to capture handlers synchronously instead of the real WAVE
 * event bus); `wos` is left real except for `callBackendService`, which is
 * spied on so other real exports this import graph needs stay intact.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
import * as wos from "@/store/wos";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));

const unnamedSubagent = {
    agent_id: "agent-1",
    slug: "s1",
    parent_agent: "parent",
    parent_block_id: "block-1",
    session_id: "session-1",
    status: "active",
    spawned_at: 0,
    last_event_at: 0,
    event_count: 1,
    model: null,
    dispatch_id: "solo:agent-1",
    display_name: null,
} as any;

const callBackendServiceSpy = vi.spyOn(wos, "callBackendService").mockImplementation(async (service, method) => {
    if (service === "subagent" && method === "ListActive") return [unnamedSubagent];
    return [];
});

import { SwarmViewModel } from "./swarm-model";

// Constructor fires `loadAll()` (async, not awaited by the constructor
// itself) — give its microtasks a turn to resolve before asserting on
// state it populates.
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("SwarmViewModel backfill-naming backlog trigger", () => {
    beforeEach(() => {
        hub.handlers.clear();
        callBackendServiceSpy.mockClear();
    });

    it("fires subagent.ResolveUnnamedBacklog exactly once on construction", () => {
        new SwarmViewModel("block-1", {} as any);

        const backlogCalls = callBackendServiceSpy.mock.calls.filter(
            (call) => call[0] === "subagent" && call[1] === "ResolveUnnamedBacklog"
        );
        expect(backlogCalls).toHaveLength(1);
        expect(backlogCalls[0]).toEqual(["subagent", "ResolveUnnamedBacklog", []]);
    });

    it("a synthetic subagent:named event still patches display_name in place (regression guard)", async () => {
        const vm = new SwarmViewModel("block-1", {} as any);
        await flush();

        expect(vm.subagentsAtom().find((s) => s.agent_id === "agent-1")?.display_name).toBeNull();

        const handler = hub.handlers.get("subagent:named");
        expect(handler).toBeDefined();
        handler!({ data: { agentId: "agent-1", displayName: "Resolved name" } });

        expect(vm.subagentsAtom().find((s) => s.agent_id === "agent-1")?.display_name).toBe("Resolved name");
    });
});
