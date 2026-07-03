// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMcpModel — view model for the agent pane's MCP Servers tab
 * (part of AgentSetupModal). Drives the list + create/edit/delete/bind
 * lifecycle over the standalone MCP Server primitive (`mcp.*` App API,
 * agentmux-srv/src/server/app_api/mcp.rs).
 *
 * `mcp.list` returns both this agent's own (non-global) servers AND every
 * global server, regardless of whether this agent is bound to it — the
 * response carries no per-row "bound to me" flag. Own (`!is_global`) rows
 * are always editable/deletable here; global rows are read-only (upsert/
 * delete on a global row is backend-FORBIDDEN) and only exposed as
 * Bind/Unbind actions, which are idempotent on the backend so the toggle
 * works correctly even without a live bound-state indicator.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

export interface McpDraft {
    id?: string;
    name: string;
    transport: string;
    config: string;
}

export function emptyMcpDraft(): McpDraft {
    return { id: undefined, name: "", transport: "stdio", config: "{}" };
}

function draftFromServer(s: McpServer): McpDraft {
    return { id: s.id, name: s.name, transport: s.transport, config: s.config };
}

export class AgentMcpModel {
    readonly agentId: string;

    private _servers = createSignal<McpServer[]>([]);
    serversAtom: Accessor<McpServer[]> = this._servers[0];
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

    selectedAtom: Accessor<McpServer | null>;

    constructor(agentId: string) {
        this.agentId = agentId;
        this.selectedAtom = createMemo(() => {
            const id = this.selectedIdAtom();
            if (!id) return null;
            return this.serversAtom().find((s) => s.id === id) ?? null;
        });
        void this.refresh();
    }

    async refresh(): Promise<void> {
        try {
            const list = await RpcApi.McpListCommand(TabRpcClient, { agent_id: this.agentId });
            this.setServers(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load MCP servers: ${(e as Error).message ?? e}`);
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

    startEdit(server: McpServer): void {
        if (server.is_global) {
            this.setError("Global MCP servers are managed in the Armory, not here.");
            return;
        }
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
            const saved = await RpcApi.McpUpsertCommand(TabRpcClient, {
                agent_id: this.agentId,
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

    async deleteServer(id: string): Promise<void> {
        const target = this.serversAtom().find((s) => s.id === id);
        if (target?.is_global) {
            this.setError("Global MCP servers are managed in the Armory, not here.");
            return;
        }
        this.setError(null);
        try {
            await RpcApi.McpDeleteCommand(TabRpcClient, { agent_id: this.agentId, id });
            if (this.selectedIdAtom() === id) this.setSelectedId(null);
            this.setDraft(null);
            await this.refresh();
        } catch (e) {
            this.setError(`Delete failed: ${(e as Error).message ?? e}`);
        }
    }

    async bind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.McpBindCommand(TabRpcClient, { agent_id: this.agentId, mcp_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Bind failed: ${(e as Error).message ?? e}`);
        }
    }

    async unbind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.McpUnbindCommand(TabRpcClient, { agent_id: this.agentId, mcp_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Unbind failed: ${(e as Error).message ?? e}`);
        }
    }

    dispose(): void {
        // Solid signals are GC'd with the instance; nothing to unsubscribe.
    }
}
