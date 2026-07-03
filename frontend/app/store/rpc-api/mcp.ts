// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// v1 composable model — standalone MCP Server primitive
// (agentmux-srv/src/server/app_api/mcp.rs). Agent-scoped: every command is
// `check_s1`-gated (ctx.agent_id must equal the request's agent_id), so
// these only work from an authenticated agent connection. The mcp.catalog.*
// commands are the window-scoped counterpart (no agent_id, global rows
// only) — that's what the Armory's MCP Servers tab uses.

import { RpcClient } from "../rpc-client";

export const McpApi = {
    McpListCommand(
        client: RpcClient,
        data: { agent_id: string },
        opts?: RpcOpts,
    ): Promise<McpServer[]> {
        return client.rpcCall("mcp.list", data, opts);
    },

    McpGetCommand(
        client: RpcClient,
        data: { agent_id: string; id: string },
        opts?: RpcOpts,
    ): Promise<McpServer | null> {
        return client.rpcCall("mcp.get", data, opts);
    },

    McpUpsertCommand(
        client: RpcClient,
        data: { agent_id: string; id?: string; name: string; transport?: string; config?: string },
        opts?: RpcOpts,
    ): Promise<McpServer> {
        return client.rpcCall("mcp.upsert", data, opts);
    },

    McpDeleteCommand(
        client: RpcClient,
        data: { agent_id: string; id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("mcp.delete", data, opts);
    },

    McpBindCommand(
        client: RpcClient,
        data: { agent_id: string; mcp_id: string },
        opts?: RpcOpts,
    ): Promise<{ bound: boolean }> {
        return client.rpcCall("mcp.bind", data, opts);
    },

    McpUnbindCommand(
        client: RpcClient,
        data: { agent_id: string; mcp_id: string },
        opts?: RpcOpts,
    ): Promise<{ unbound: boolean }> {
        return client.rpcCall("mcp.unbind", data, opts);
    },

    // ── Armory catalog (global servers only, no agent_id) ──────────────────

    McpCatalogListCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<McpServer[]> {
        return client.rpcCall("mcp.catalog.list", data, opts);
    },

    McpCatalogUpsertCommand(
        client: RpcClient,
        data: { id?: string; name: string; transport?: string; config?: string },
        opts?: RpcOpts,
    ): Promise<McpServer> {
        return client.rpcCall("mcp.catalog.upsert", data, opts);
    },

    McpCatalogDeleteCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("mcp.catalog.delete", data, opts);
    },
};
