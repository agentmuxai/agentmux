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
    ): Promise<McpServerListItem[]> {
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

    /** Health/prerequisite probe — see McpProbeResult (gotypes.d.ts). */
    McpProbeCommand(
        client: RpcClient,
        data: { agent_id: string; id: string },
        opts?: RpcOpts,
    ): Promise<McpProbeResult> {
        return client.rpcCall("mcp.probe", data, opts);
    },

    // ── Armory catalog (global servers only, no agent_id) ──────────────────

    McpCatalogListCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<McpServerCatalogItem[]> {
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

    /** Health/prerequisite probe for a global catalog server — no agent_id
     *  (mirrors mcp.catalog.*'s window-scoped shape). See McpProbeResult. */
    McpCatalogProbeCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<McpProbeResult> {
        return client.rpcCall("mcp.catalog.probe", data, opts);
    },

    // Catalog-tier sibling of McpBindCommand — no agent_id/check_s1 gate,
    // since the Armory's connection is never agent-authenticated and can
    // never satisfy McpBindCommand's check_s1. See
    // docs/reports/REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md.
    McpCatalogBindCommand(
        client: RpcClient,
        data: { agent_id: string; mcp_id: string },
        opts?: RpcOpts,
    ): Promise<{ bound: boolean }> {
        return client.rpcCall("mcp.catalog.bind", data, opts);
    },

    // Catalog-tier siblings of McpListCommand / McpUnbindCommand — no
    // agent_id/check_s1 gate on the *caller*, but agent_id is still required
    // in the payload (whose bindings to list/unbind). Used by
    // AgentStashModal's MCP Servers tab, which runs over the dashboard's
    // connection and can never satisfy McpListCommand/McpUnbindCommand's
    // check_s1.
    McpCatalogListForAgentCommand(
        client: RpcClient,
        data: { agent_id: string },
        opts?: RpcOpts,
    ): Promise<McpServerListItem[]> {
        return client.rpcCall("mcp.catalog.list_for_agent", data, opts);
    },

    McpCatalogUnbindCommand(
        client: RpcClient,
        data: { agent_id: string; mcp_id: string },
        opts?: RpcOpts,
    ): Promise<{ unbound: boolean }> {
        return client.rpcCall("mcp.catalog.unbind", data, opts);
    },

    // ── Bundle-scoped siblings (composable model v2) ────────────────────
    // docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md, GH issue #2024
    // item 3. Same no-agent_id/no-check_s1 shape as the catalog trio above,
    // keyed by bundle_id instead of agent_id — only global servers (or ones
    // already bundle-bound) may be bound, same trust boundary as
    // McpCatalogBindCommand.

    McpCatalogBindToBundleCommand(
        client: RpcClient,
        data: { bundle_id: string; mcp_id: string },
        opts?: RpcOpts,
    ): Promise<{ bound: boolean }> {
        return client.rpcCall("mcp.catalog.bind_to_bundle", data, opts);
    },

    McpCatalogUnbindFromBundleCommand(
        client: RpcClient,
        data: { bundle_id: string; mcp_id: string },
        opts?: RpcOpts,
    ): Promise<{ unbound: boolean }> {
        return client.rpcCall("mcp.catalog.unbind_from_bundle", data, opts);
    },

    McpCatalogListForBundleCommand(
        client: RpcClient,
        data: { bundle_id: string },
        opts?: RpcOpts,
    ): Promise<McpServerBundleListItem[]> {
        return client.rpcCall("mcp.catalog.list_for_bundle", data, opts);
    },

    // Creates a NEW, PRIVATE server scoped directly to a bundle (never
    // global) — the actual "give this bundle its own tool" path.
    // McpCatalogBindToBundleCommand alone can only reference already-global
    // rows, which have no effect once bound (already unconditionally
    // visible to every agent).
    McpCatalogUpsertForBundleCommand(
        client: RpcClient,
        data: { bundle_id: string; id?: string; name: string; transport?: string; config?: string },
        opts?: RpcOpts,
    ): Promise<McpServer> {
        return client.rpcCall("mcp.catalog.upsert_for_bundle", data, opts);
    },
};
