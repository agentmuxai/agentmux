// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for BundleMcpModel — the bundle-scoped MCP Servers view model
// (composable model v2, docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md).

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        McpCatalogListForBundleCommand: vi.fn(),
        McpCatalogBindToBundleCommand: vi.fn(),
        McpCatalogUnbindFromBundleCommand: vi.fn(),
        McpCatalogUpsertForBundleCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import { BundleMcpModel } from "./bundle-mcp-model";

const server = (id: string, overrides: Partial<McpServerBundleListItem> = {}): McpServerBundleListItem => ({
    id,
    name: `srv-${id}`,
    transport: "stdio",
    config: "{}",
    is_global: true,
    created_at: 0,
    updated_at: 0,
    bound_to_bundle: false,
    ...overrides,
});

let dispose: (() => void) | undefined;

beforeEach(() => {
    vi.clearAllMocks();
    hub.handlers.clear();
});

afterEach(() => {
    dispose?.();
    dispose = undefined;
});

function makeModel(bundleId = "bundle-1"): BundleMcpModel {
    let model!: BundleMcpModel;
    createRoot((d) => {
        dispose = d;
        model = new BundleMcpModel(bundleId);
    });
    return model;
}

describe("BundleMcpModel", () => {
    test("refresh loads the bundle-scoped list on construction", async () => {
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([server("s1")]);
        const model = makeModel("bundle-1");
        await model.refresh();

        expect(RpcApi.McpCatalogListForBundleCommand).toHaveBeenCalledWith(
            {},
            { bundle_id: "bundle-1" },
        );
        expect(model.serversAtom()).toHaveLength(1);
    });

    test("a refresh failure surfaces via errorAtom, not a thrown exception", async () => {
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockRejectedValue(new Error("boom"));
        const model = makeModel();
        await model.refresh();

        expect(model.errorAtom()).toContain("boom");
        expect(model.serversAtom()).toEqual([]);
    });

    test("bind calls McpCatalogBindToBundleCommand with the bundle_id, then refreshes", async () => {
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([]);
        vi.mocked(RpcApi.McpCatalogBindToBundleCommand).mockResolvedValue({ bound: true });
        const model = makeModel("bundle-1");
        await model.refresh();
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([server("s1", { bound_to_bundle: true })]);

        await model.bind("s1");

        expect(RpcApi.McpCatalogBindToBundleCommand).toHaveBeenCalledWith(
            {},
            { bundle_id: "bundle-1", mcp_id: "s1" },
        );
        expect(model.serversAtom()[0]?.bound_to_bundle).toBe(true);
    });

    test("unbind calls McpCatalogUnbindFromBundleCommand with the bundle_id", async () => {
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([]);
        vi.mocked(RpcApi.McpCatalogUnbindFromBundleCommand).mockResolvedValue({ unbound: true });
        const model = makeModel("bundle-1");
        await model.refresh();

        await model.unbind("s1");

        expect(RpcApi.McpCatalogUnbindFromBundleCommand).toHaveBeenCalledWith(
            {},
            { bundle_id: "bundle-1", mcp_id: "s1" },
        );
    });

    test("a bind failure surfaces via errorAtom instead of throwing", async () => {
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([]);
        vi.mocked(RpcApi.McpCatalogBindToBundleCommand).mockRejectedValue(new Error("FORBIDDEN"));
        const model = makeModel();
        await model.refresh();

        await model.bind("s1");

        expect(model.errorAtom()).toContain("FORBIDDEN");
    });

    test("addPrivate creates a new server scoped to this bundle, then refreshes", async () => {
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([]);
        vi.mocked(RpcApi.McpCatalogUpsertForBundleCommand).mockResolvedValue(
            server("new-1", { is_global: false, bound_to_bundle: true }) as any,
        );
        const model = makeModel("bundle-1");
        await model.refresh();
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([
            server("new-1", { is_global: false, bound_to_bundle: true }),
        ]);

        const ok = await model.addPrivate("My Tool", '{"command":"my-tool"}');

        expect(ok).toBe(true);
        expect(RpcApi.McpCatalogUpsertForBundleCommand).toHaveBeenCalledWith(
            {},
            { bundle_id: "bundle-1", name: "My Tool", config: '{"command":"my-tool"}' },
        );
        expect(model.addingAtom()).toBe(false);
        expect(model.serversAtom()).toHaveLength(1);
    });

    test("addPrivate sets addingAtom while the call is in flight", async () => {
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([]);
        let resolveUpsert!: (v: McpServer) => void;
        vi.mocked(RpcApi.McpCatalogUpsertForBundleCommand).mockReturnValue(
            new Promise((resolve) => { resolveUpsert = resolve; }) as any,
        );
        const model = makeModel();
        await model.refresh();

        const promise = model.addPrivate("name", "{}");
        expect(model.addingAtom()).toBe(true);
        resolveUpsert(server("x") as any);
        await promise;
        expect(model.addingAtom()).toBe(false);
    });

    test("a duplicate-name addPrivate failure surfaces via errorAtom and clears addingAtom", async () => {
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([]);
        vi.mocked(RpcApi.McpCatalogUpsertForBundleCommand).mockRejectedValue(
            new Error("server name 'X' already bound to this bundle"),
        );
        const model = makeModel();
        await model.refresh();

        const ok = await model.addPrivate("X", "{}");

        expect(ok).toBe(false);
        expect(model.errorAtom()).toContain("already bound to this bundle");
        expect(model.addingAtom()).toBe(false);
    });

    test("selectedAtom resolves the currently selected server by id", async () => {
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([server("s1"), server("s2")]);
        const model = makeModel();
        await model.refresh();

        model.handleSelect(model.serversAtom()[1]!);
        expect(model.selectedAtom()?.id).toBe("s2");
    });

    test("an mcp:changed event fired elsewhere triggers a refresh", async () => {
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([]);
        const model = makeModel("bundle-1");
        await model.refresh();
        vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([server("s1")]);

        hub.handlers.get("mcp:changed")?.({});
        await vi.waitFor(() => expect(model.serversAtom()).toHaveLength(1));
    });

    test("dispose unsubscribes from mcp:changed", () => {
        const model = makeModel();
        expect(hub.handlers.has("mcp:changed")).toBe(true);
        model.dispose();
        expect(hub.handlers.has("mcp:changed")).toBe(false);
    });
});
