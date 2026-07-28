// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMcpModel — view model for the agent pane's MCP Servers tab (part of
 * AgentStashModal). A reactive, read-only view of the standalone MCP
 * Server primitive (`mcp.*` App API, agentmux-srv/src/server/app_api/mcp.rs)
 * plus a Bind/Unbind toggle — NOT a create/edit/delete surface. Global
 * servers are authored in the Armory; this tab shows every server visible
 * to this agent (global + this agent's own, if any were created via its
 * own tool calls) and lets you choose which global ones apply here.
 *
 * `mcp.catalog.list_for_agent` returns both this agent's own (non-global)
 * servers AND every global server, each annotated with `bound_to_agent` —
 * whether *this* agent specifically holds the bind ref (own rows are
 * always true; a global row may or may not be). Unlike `mcp.list`, this
 * and the bind/unbind commands below carry no `check_s1` gate: they were
 * originally check_s1-gated (agent-self-service only), which meant every
 * action in this tab failed "unauthorized" when opened from the dashboard
 * — this tab's connection is never agent-authenticated. See
 * docs/reports/REPORT_ARMORY_ARCHITECTURE_AND_NAMING_REVIEW_2026_07_23.md §2.2.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";

export class AgentMcpModel {
    readonly agentId: string;

    private _servers = createSignal<McpServerListItem[]>([]);
    serversAtom: Accessor<McpServerListItem[]> = this._servers[0];
    private setServers = this._servers[1];

    private _selectedId = createSignal<string | null>(null);
    selectedIdAtom: Accessor<string | null> = this._selectedId[0];
    setSelectedId = this._selectedId[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    selectedAtom: Accessor<McpServerListItem | null>;

    // Cross-window reactivity — a bind/unbind or Armory catalog edit made
    // elsewhere (another window, or this same agent's own tool calls)
    // refreshes this view without a manual reopen.
    private unsubChanged: () => void;

    constructor(agentId: string) {
        this.agentId = agentId;
        this.selectedAtom = createMemo(() => {
            const id = this.selectedIdAtom();
            if (!id) return null;
            return this.serversAtom().find((s) => s.id === id) ?? null;
        });
        void this.refresh();
        this.unsubChanged = waveEventSubscribe({
            eventType: "mcp:changed",
            handler: () => void this.refresh(),
        });
    }

    async refresh(): Promise<void> {
        try {
            const list = await RpcApi.McpCatalogListForAgentCommand(TabRpcClient, { agent_id: this.agentId });
            this.setServers(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load MCP servers: ${(e as Error).message ?? e}`);
        }
    }

    handleSelect(server: McpServerListItem): void {
        this.setError(null);
        this.setSelectedId(server.id);
    }

    async bind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.McpCatalogBindCommand(TabRpcClient, { agent_id: this.agentId, mcp_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Bind failed: ${(e as Error).message ?? e}`);
        }
    }

    async unbind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.McpCatalogUnbindCommand(TabRpcClient, { agent_id: this.agentId, mcp_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Unbind failed: ${(e as Error).message ?? e}`);
        }
    }

    dispose(): void {
        this.unsubChanged();
    }
}
