// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for the mcp-capabilities store — the MCP-server analogue of
// toolchain-capabilities.ts, backed by mcp.catalog.probe.

import { describe, test, expect, beforeEach, afterEach, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        McpCatalogProbeCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import {
    ensureMcpCapability,
    refreshMcpCapability,
    getMcpCapability,
    isMcpConnected,
    watchMcpCapability,
    resetMcpCapabilities,
} from "./mcp-capabilities";

beforeEach(() => {
    vi.clearAllMocks();
    resetMcpCapabilities();
});

describe("probe result mapping", () => {
    test("a connected server reports status + tool count", async () => {
        vi.mocked(RpcApi.McpCatalogProbeCommand).mockResolvedValue({
            status: "connected", tool_count: 12, server_name: "AbletonMCP", server_version: "1.0.0", error: null,
        } as any);

        const result = await ensureMcpCapability("srv-1");

        expect(result.status).toBe("connected");
        expect(result.toolCount).toBe(12);
        expect(result.serverName).toBe("AbletonMCP");
        expect(isMcpConnected("srv-1")).toBe(true);
    });

    test("a handshake failure (e.g. Ableton not running) is not the same as unreachable", async () => {
        vi.mocked(RpcApi.McpCatalogProbeCommand).mockResolvedValue({
            status: "handshake_failed", tool_count: null, server_name: null, server_version: null,
            error: "no response within 8s",
        } as any);

        const result = await ensureMcpCapability("srv-ableton");

        expect(result.status).toBe("handshake_failed");
        expect(isMcpConnected("srv-ableton")).toBe(false);
    });

    test("RPC rejection maps to unreachable, not a thrown error", async () => {
        vi.mocked(RpcApi.McpCatalogProbeCommand).mockRejectedValue(new Error("connection reset"));

        const result = await ensureMcpCapability("srv-2");

        expect(result.status).toBe("unreachable");
        expect(result.error).toBe("connection reset");
    });
});

describe("caching + concurrent-call de-duplication", () => {
    test("two concurrent ensureMcpCapability calls for the same id share one RPC", async () => {
        let resolveRpc!: (v: any) => void;
        vi.mocked(RpcApi.McpCatalogProbeCommand).mockReturnValue(
            new Promise((resolve) => { resolveRpc = resolve; }) as any,
        );

        const p1 = ensureMcpCapability("srv-3");
        const p2 = ensureMcpCapability("srv-3");
        resolveRpc({ status: "connected", tool_count: 3, server_name: null, server_version: null, error: null });

        const [r1, r2] = await Promise.all([p1, p2]);
        expect(r1).toEqual(r2);
        expect(RpcApi.McpCatalogProbeCommand).toHaveBeenCalledTimes(1);
    });

    test("a second call after completion reuses the cached result without a new RPC", async () => {
        vi.mocked(RpcApi.McpCatalogProbeCommand).mockResolvedValue({
            status: "connected", tool_count: 1, server_name: null, server_version: null, error: null,
        } as any);

        await ensureMcpCapability("srv-4");
        await ensureMcpCapability("srv-4");

        expect(RpcApi.McpCatalogProbeCommand).toHaveBeenCalledTimes(1);
    });

    test("refreshMcpCapability bypasses the cache and re-probes", async () => {
        vi.mocked(RpcApi.McpCatalogProbeCommand)
            .mockResolvedValueOnce({ status: "unreachable", tool_count: null, server_name: null, server_version: null, error: "e1" } as any)
            .mockResolvedValueOnce({ status: "connected", tool_count: 5, server_name: null, server_version: null, error: null } as any);

        await ensureMcpCapability("srv-5");
        const refreshed = await refreshMcpCapability("srv-5");

        expect(refreshed.status).toBe("connected");
        expect(RpcApi.McpCatalogProbeCommand).toHaveBeenCalledTimes(2);
    });
});

describe("watchMcpCapability polling", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    test("polls at the given interval until stopped", async () => {
        vi.mocked(RpcApi.McpCatalogProbeCommand).mockResolvedValue({
            status: "connected", tool_count: 1, server_name: null, server_version: null, error: null,
        } as any);

        const stop = watchMcpCapability("srv-6", 1000);
        await vi.advanceTimersByTimeAsync(0); // flush the immediate ensure call
        expect(RpcApi.McpCatalogProbeCommand).toHaveBeenCalledTimes(1);

        await vi.advanceTimersByTimeAsync(1000);
        expect(RpcApi.McpCatalogProbeCommand).toHaveBeenCalledTimes(2);

        stop();
        await vi.advanceTimersByTimeAsync(2000);
        expect(RpcApi.McpCatalogProbeCommand).toHaveBeenCalledTimes(2);
    });
});

test("getMcpCapability returns unknown for a server that was never probed", () => {
    expect(getMcpCapability("never-probed")).toEqual({ status: "unknown" });
});
