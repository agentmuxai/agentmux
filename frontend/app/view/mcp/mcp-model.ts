// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * McpCatalogModel — view model for Connectors → MCP servers. Drives
 * list + create/edit/delete, plus binding a global server to an agent,
 * over the window-scoped `mcp.catalog.*` MuxAPI
 * (crates/srv/src/server/app_api/mcp.rs) — no `agent_id`/`check_s1`
 * context, since Connectors' connection is never agent-authenticated.
 * Every row here is global by construction — the catalog only ever
 * lists/creates/edits is_global rows. Per-agent private servers, and
 * unbinding, live in the Agent-setup modal (AgentMcpModel), not here.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { onMuxEvent } from "@/app/store/mps";
import type { McpPreloadEntry } from "./mcp-preload-catalog";
import type { AgentDefinition } from "@/app/store/rpc-api";

import { WpsEvent } from "@/app/store/mps-events";
import type { McpServer } from "@/types/rpc/McpServer";
import type { McpServerCatalogItem } from "@/types/rpc/McpServerCatalogItem";
export interface McpDraft {
    id?: string;
    name: string;
    transport: string;
    config: string;
    /** The preset it was started from: the form shows that preset's notes,
     *  its risk above all, before Save (saving starts probing the server). */
    presetId?: string;
}

function emptyMcpDraft(): McpDraft {
    return { id: undefined, name: "", transport: "stdio", config: "{}" };
}

function draftFromServer(s: McpServer): McpDraft {
    return { id: s.id, name: s.name, transport: s.transport, config: s.config };
}

/** Who a catalog server reaches, in a few words: everyone, or how many
 *  agents it is bound to. */
export function reachLabel(server: McpServerCatalogItem): string {
    if (server.all_agents) return "All agents";
    return `${server.bound_count} ${server.bound_count === 1 ? "agent" : "agents"}`;
}

/** A card's subtitle: the transport and what it runs or reaches ("stdio ·
 *  uvx", "http · mcp.example.com/sse"). Only the command's name, or the
 *  URL's host and path: arguments, env, headers and a URL's query or
 *  user info can hold secrets. */
export function connectionSummary(transport: string, config: string): string {
    let target = "";
    try {
        const c = JSON.parse(config || "{}") as { command?: unknown; url?: unknown };
        if (typeof c?.command === "string") target = commandName(c.command);
        else if (typeof c?.url === "string") target = urlSummary(c.url);
    } catch {
        // Not JSON: the transport alone.
    }
    return [transport || "stdio", target].filter(Boolean).join(" · ");
}

function commandName(command: string): string {
    // Up to the first flag or assignment, then the program's file name.
    const cmd = command.trim().split(/\s+(?=-|\S*=)/)[0];
    const isPath = /^([a-zA-Z]:)?[\\/]/.test(cmd);
    const name = cmd.split(/[\\/]/).pop() || cmd;
    return isPath ? name : name.split(/\s+/)[0];
}

function urlSummary(url: string): string {
    try {
        const u = new URL(url);
        return u.host + (u.pathname === "/" ? "" : u.pathname);
    } catch {
        return "";
    }
}

export class McpCatalogModel {
    // Cross-window reactivity (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md) —
    // a bind/unbind/add/edit made elsewhere refreshes Connectors →
    // MCP servers without a manual reopen.
    private unsubChanged: () => void;

    private _servers = createSignal<McpServerCatalogItem[]>([]);
    serversAtom: Accessor<McpServerCatalogItem[]> = this._servers[0];
    private setServers = this._servers[1];

    private _selectedId = createSignal<string | null>(null);
    selectedIdAtom: Accessor<string | null> = this._selectedId[0];
    setSelectedId = this._selectedId[1];

    private _draft = createSignal<McpDraft | null>(null);
    draftAtom: Accessor<McpDraft | null> = this._draft[0];
    setDraft = this._draft[1];

    private _saving = createSignal<boolean>(false);
    savingAtom: Accessor<boolean> = this._saving[0];
    private setSaving = this._saving[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    // Catalog-side bind action (#1960 gap #3) — the agent picker's options,
    // and the id the user currently has selected in it.
    private _agents = createSignal<AgentDefinition[]>([]);
    agentsAtom: Accessor<AgentDefinition[]> = this._agents[0];
    private setAgents = this._agents[1];

    private _bindAgentId = createSignal<string>("");
    bindAgentIdAtom: Accessor<string> = this._bindAgentId[0];
    setBindAgentId = this._bindAgentId[1];

    selectedAtom: Accessor<McpServerCatalogItem | null>;

    constructor() {
        this.selectedAtom = createMemo(() => {
            const id = this.selectedIdAtom();
            if (!id) return null;
            return this.serversAtom().find((s) => s.id === id) ?? null;
        });
        void this.refresh();
        void this.loadAgents();
        this.unsubChanged = onMuxEvent(WpsEvent.McpChanged, () => void this.refresh());
    }

    async refresh(): Promise<void> {
        try {
            const list = await RpcApi.McpCatalogListCommand(TabRpcClient, {});
            this.setServers(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load MCP servers: ${(e as Error).message ?? e}`);
        }
    }

    // Populates the catalog-side bind picker. Best-effort: a failure here
    // shouldn't block viewing/editing the catalog, only disable binding.
    async loadAgents(): Promise<void> {
        try {
            const agents = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient);
            this.setAgents(agents);
        } catch {
            this.setAgents([]);
        }
    }

    async bindToAgent(serverId: string, agentId: string): Promise<void> {
        if (!agentId) {
            this.setError("Pick an agent to bind to.");
            return;
        }
        this.setError(null);
        try {
            await RpcApi.McpCatalogBindCommand(TabRpcClient, { agent_id: agentId, mcp_id: serverId });
            await this.refresh();
        } catch (e) {
            this.setError(`Bind failed: ${(e as Error).message ?? e}`);
        }
    }

    /** Who a catalog server reaches beyond its binds: every agent, and
     *  container agents too (SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md §2.3). */
    async setReach(serverId: string, reach: { all_agents: boolean; in_containers: boolean }): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.McpCatalogSetReachCommand(TabRpcClient, { id: serverId, ...reach });
            await this.refresh();
        } catch (e) {
            this.setError(`Saving who gets it failed: ${(e as Error).message ?? e}`);
        }
    }

    handleSelect(server: McpServer): void {
        this.setError(null);
        this.setDraft(null);
        this.setSelectedId(server.id);
    }

    startNew(): void {
        this.setError(null);
        this.setDraft(emptyMcpDraft());
        this.setSelectedId(null);
    }

    /** A preset chip in the Add gallery — pre-fill the normal draft form, name included
     *  (unlike a from-scratch draft, the name is meaningful and rarely
     *  needs editing) — and drop straight into it, same form as "+ New". */
    startFromCatalog(entry: McpPreloadEntry): void {
        this.setError(null);
        this.setSelectedId(null);
        this.setDraft({
            id: undefined,
            name: entry.name,
            transport: entry.transport,
            config: JSON.stringify(entry.config, null, 2),
            presetId: entry.id,
        });
    }

    startEdit(server: McpServer): void {
        this.setError(null);
        this.setDraft(draftFromServer(server));
        this.setSelectedId(server.id);
    }

    cancelDraft(): void {
        this.setDraft(null);
        this.setError(null);
    }

    async saveDraft(): Promise<void> {
        const draft = this.draftAtom();
        if (!draft) return;
        if (!draft.name.trim()) {
            this.setError("Name is required.");
            return;
        }
        try {
            JSON.parse(draft.config || "{}");
        } catch {
            this.setError("Config must be valid JSON.");
            return;
        }
        this.setSaving(true);
        this.setError(null);
        try {
            const saved = await RpcApi.McpCatalogUpsertCommand(TabRpcClient, {
                id: draft.id,
                name: draft.name.trim(),
                transport: draft.transport || "stdio",
                config: draft.config || "{}",
            });
            await this.refresh();
            this.setDraft(null);
            this.setSelectedId(saved.id);
        } catch (e) {
            this.setError(`Save failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setSaving(false);
        }
    }

    /** The agents a server is bound to, by name, for the delete
     *  confirmation. The catalog only counts them, so each agent is asked;
     *  one that can't be asked is left out. */
    async boundAgentNames(server: McpServerCatalogItem): Promise<string[]> {
        if (server.bound_count === 0) return [];
        const names = await Promise.all(
            this.agentsAtom().map(async (agent) => {
                try {
                    const list = await RpcApi.McpCatalogListForAgentCommand(TabRpcClient, { agent_id: agent.id });
                    return list.some((s) => s.id === server.id && s.bound_to_agent) ? agent.name : null;
                } catch {
                    return null;
                }
            }),
        );
        return names.filter((n): n is string => n !== null).sort((a, b) => a.localeCompare(b));
    }

    async deleteServer(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.McpCatalogDeleteCommand(TabRpcClient, { id });
            if (this.selectedIdAtom() === id) this.setSelectedId(null);
            this.setDraft(null);
            await this.refresh();
        } catch (e) {
            this.setError(`Delete failed: ${(e as Error).message ?? e}`);
        }
    }

    dispose(): void {
        this.unsubChanged();
    }
}
