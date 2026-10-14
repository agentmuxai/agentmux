// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// McpCatalogModel — the Connectors → MCP servers model. Scoped
// to the reactive-updates wiring added by
// docs/specs/SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md; this model had no
// prior test coverage, and full CRUD coverage is out of scope here.

import { beforeEach, describe, expect, test, vi } from "vitest";

const mcpCatalogListMock = vi.fn().mockResolvedValue([]);
const listAgentDefinitionsMock = vi.fn().mockResolvedValue([]);
const setReachMock = vi.fn().mockResolvedValue({ all_agents: true, in_containers: false });
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        McpCatalogListCommand: (...args: unknown[]) => mcpCatalogListMock(...args),
        ListAgentDefinitionsCommand: (...args: unknown[]) => listAgentDefinitionsMock(...args),
        McpCatalogSetReachCommand: (...args: unknown[]) => setReachMock(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

// Same hub pattern bundle-skill-model.test.ts / global-bundle-model.test.ts use.
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

import { McpCatalogModel, reachLabel } from "./mcp-model";
import type { McpServerCatalogItem } from "@/types/rpc/McpServerCatalogItem";

beforeEach(() => {
    mcpCatalogListMock.mockClear();
    mcpCatalogListMock.mockResolvedValue([]);
    listAgentDefinitionsMock.mockClear();
    listAgentDefinitionsMock.mockResolvedValue([]);
    mpsHub.handlers.clear();
});

describe("McpCatalogModel — reactive updates", () => {
    test("subscribes to mcp:changed and refreshes on it", async () => {
        const model = new McpCatalogModel();
        await Promise.resolve();
        mcpCatalogListMock.mockClear();

        mpsHub.handlers.get("mcp:changed")?.({});
        await Promise.resolve();

        expect(mcpCatalogListMock).toHaveBeenCalledTimes(1);
    });

    test("an unrelated event type does not trigger a refresh", async () => {
        new McpCatalogModel();
        await Promise.resolve();
        mcpCatalogListMock.mockClear();

        mpsHub.handlers.get("skills:changed")?.({});
        await Promise.resolve();

        expect(mcpCatalogListMock).not.toHaveBeenCalled();
    });

    test("unsubscribes on dispose", async () => {
        const model = new McpCatalogModel();
        await Promise.resolve();
        expect(mpsHub.handlers.has("mcp:changed")).toBe(true);

        model.dispose();

        expect(mpsHub.handlers.has("mcp:changed")).toBe(false);
    });
});

describe("McpCatalogModel — reach", () => {
    test("setReach sends both switches and reloads the list", async () => {
        const model = new McpCatalogModel();
        await Promise.resolve();
        mcpCatalogListMock.mockClear();
        setReachMock.mockClear();

        await model.setReach("srv-1", { all_agents: true, in_containers: false });

        expect(setReachMock).toHaveBeenCalledWith({}, { id: "srv-1", all_agents: true, in_containers: false });
        expect(mcpCatalogListMock).toHaveBeenCalledTimes(1);
        expect(model.errorAtom()).toBeNull();
    });

    test("a failed save says so", async () => {
        const model = new McpCatalogModel();
        setReachMock.mockRejectedValueOnce(new Error("FORBIDDEN"));

        await model.setReach("srv-1", { all_agents: false, in_containers: true });

        expect(model.errorAtom()).toContain("FORBIDDEN");
    });
});

describe("reachLabel", () => {
    const item = (over: Partial<McpServerCatalogItem>): McpServerCatalogItem => ({
        id: "s",
        name: "git",
        transport: "stdio",
        config: "{}",
        is_global: true,
        created_at: 1,
        updated_at: 1,
        bound_count: 0,
        all_agents: false,
        in_containers: false,
        ...over,
    });

    test("says who gets a catalog server", () => {
        expect(reachLabel(item({ all_agents: true, bound_count: 2 }))).toBe("All agents");
        expect(reachLabel(item({ bound_count: 1 }))).toBe("1 agent");
        expect(reachLabel(item({ bound_count: 0 }))).toBe("0 agents");
    });
});
