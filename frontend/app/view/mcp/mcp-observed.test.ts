// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, test, vi } from "vitest";

const observedListMock = vi.fn().mockResolvedValue([]);
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { McpObservedListCommand: (...args: unknown[]) => observedListMock(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const reconnect = vi.hoisted(() => ({ handlers: [] as (() => void)[] }));
vi.mock("@/app/store/ws", () => ({
    addWSReconnectHandler: (h: () => void) => reconnect.handlers.push(h),
}));
const mpsHub = vi.hoisted(() => ({ handlers: new Map<string, (e: unknown) => void>() }));
vi.mock("@/app/store/mps", async () => {
    const { isNoPayloadMuxEvent } = await vi.importActual<typeof import("@/app/store/mps-events.generated")>("@/app/store/mps-events.generated");
    const mock = {
        muxEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
            mpsHub.handlers.set(sub.eventType, sub.handler);
            return () => mpsHub.handlers.delete(sub.eventType);
        }),
    };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); else if (isNoPayloadMuxEvent(eventType)) handler(null, e); } } as any);
    return { ...mock, onMuxEvent };
});

import { groupObserved, McpObservedModel, notLoadedBy, notLoadedText, rowSummary, sourceLabel } from "./mcp-observed";

const agents = [
    {
        block_id: "b2",
        label: "Korp",
        at_ms: 2,
        missing: ["git"],
        servers: [
            { name: "agentmux", status: "connected", source: "builtin", tools: 95 },
            { name: "claude.ai Gmail", status: "needs-auth", source: "claudeai", tools: 0 },
        ],
    },
    {
        block_id: "b1",
        label: "AgentY",
        at_ms: 1,
        missing: [],
        servers: [
            { name: "git", status: "failed", source: "catalog", tools: 0 },
            { name: "agentmux", status: "connected", source: "builtin", tools: 95 },
            { name: "claude.ai Gmail", status: "connected", source: "claudeai", tools: 6 },
        ],
    },
];

beforeEach(() => {
    observedListMock.mockReset();
    observedListMock.mockResolvedValue([]);
    mpsHub.handlers.clear();
});

describe("groupObserved", () => {
    test("one row per server, AgentMux's own first, each with its agents by name", () => {
        const rows = groupObserved(agents);
        expect(rows.map((r) => `${r.source}:${r.name}`)).toEqual([
            "builtin:agentmux",
            "catalog:git",
            "claudeai:claude.ai Gmail",
        ]);
        expect(rows[0].uses.map((u) => u.agent)).toEqual(["AgentY", "Korp"]);
        expect(rows[2].uses.map((u) => `${u.agent}:${u.status}`)).toEqual(["AgentY:connected", "Korp:needs-auth"]);
    });

    test("a summary counts the agents and the ones not connected", () => {
        const [agentmux, git, gmail] = groupObserved(agents);
        expect(rowSummary(agentmux)).toBe("2 agents");
        expect(rowSummary(git)).toBe("1 agent, 1 not connected");
        expect(rowSummary(gmail)).toBe("2 agents, 1 not connected");
    });

    test("sources read as words", () => {
        expect(sourceLabel("builtin")).toBe("Built in");
        expect(sourceLabel("claudeai")).toBe("claude.ai connector");
        expect(sourceLabel("user")).toBe("Account config");
        expect(sourceLabel("something-new")).toBe("something-new");
    });
});

describe("not loaded", () => {
    test("names the agents that should have a server but don't, by name", () => {
        expect(notLoadedBy(agents, "git")).toEqual(["Korp"]);
        expect(notLoadedBy(agents, "fetch")).toEqual([]);
    });

    test("says so in a sentence, and cuts a long list", () => {
        expect(notLoadedText([])).toBeNull();
        expect(notLoadedText(["Korp"])).toBe("Not loaded by Korp. Restart it to apply.");
        expect(notLoadedText(["AgentY", "Korp"])).toBe("Not loaded by AgentY and Korp. Restart them to apply.");
        expect(notLoadedText(["A", "B", "C"])).toBe("Not loaded by A, B and C. Restart them to apply.");
        expect(notLoadedText(["A", "B", "C", "D", "E"])).toBe(
            "Not loaded by A, B, C and 2 more. Restart them to apply."
        );
    });
});

describe("McpObservedModel", () => {
    test("loads, and reloads when a report or the catalog changes", async () => {
        observedListMock.mockResolvedValue(agents);
        const model = new McpObservedModel();
        await vi.waitFor(() => expect(model.rowsAtom()).toHaveLength(3));
        expect(model.loadedAtom()).toBe(true);
        expect(notLoadedBy(model.agentsAtom(), "git")).toEqual(["Korp"]);

        observedListMock.mockClear();
        mpsHub.handlers.get("mcp:observed")?.({ data: { block_id: "b2" } });
        mpsHub.handlers.get("mcp:changed")?.({});
        expect(observedListMock).toHaveBeenCalledTimes(2);
        model.dispose();
        expect(mpsHub.handlers.size).toBe(0);
    });

    test("reloads when the connection to srv comes back, until disposed", async () => {
        const model = new McpObservedModel();
        await vi.waitFor(() => expect(model.loadedAtom()).toBe(true));
        expect(reconnect.handlers).toHaveLength(1);
        observedListMock.mockClear();
        reconnect.handlers[0]();
        expect(observedListMock).toHaveBeenCalledTimes(1);
        model.dispose();
        observedListMock.mockClear();
        reconnect.handlers[0]();
        expect(observedListMock).not.toHaveBeenCalled();
    });

    test("an srv that sends no missing list reads as nothing missing", async () => {
        const { missing: _omitted, ...older } = agents[0];
        observedListMock.mockResolvedValueOnce([older]);
        const model = new McpObservedModel();
        await vi.waitFor(() => expect(model.agentsAtom()).toHaveLength(1));
        expect(notLoadedBy(model.agentsAtom(), "git")).toEqual([]);
    });

    test("an older reply never overwrites a newer one", async () => {
        let resolveFirst: (v: unknown) => void = () => {};
        observedListMock.mockImplementationOnce(() => new Promise((r) => (resolveFirst = r)));
        observedListMock.mockResolvedValueOnce(agents);
        const model = new McpObservedModel();
        await model.refresh();
        expect(model.rowsAtom()).toHaveLength(3);
        resolveFirst([]);
        await Promise.resolve();
        await Promise.resolve();
        expect(model.rowsAtom()).toHaveLength(3);
    });

    test("a failed refresh keeps the rows shown", async () => {
        observedListMock.mockResolvedValueOnce(agents);
        const model = new McpObservedModel();
        await vi.waitFor(() => expect(model.rowsAtom()).toHaveLength(3));
        observedListMock.mockRejectedValueOnce(new Error("socket closed"));
        await model.refresh();
        expect(model.rowsAtom()).toHaveLength(3);
    });

    test("reconnected to an srv without the command, the old rows go", async () => {
        observedListMock.mockResolvedValueOnce(agents);
        const model = new McpObservedModel();
        await vi.waitFor(() => expect(model.rowsAtom()).toHaveLength(3));
        observedListMock.mockRejectedValueOnce(new Error("unknown command: mcp.observed.list"));
        await model.refresh();
        expect(model.rowsAtom()).toEqual([]);
        expect(model.agentsAtom()).toEqual([]);
    });

    test("an srv without the command shows nothing, not an error", async () => {
        observedListMock.mockRejectedValue(new Error("unknown command"));
        const model = new McpObservedModel();
        await vi.waitFor(() => expect(model.loadedAtom()).toBe(true));
        expect(model.rowsAtom()).toEqual([]);
    });
});
