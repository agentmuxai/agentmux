// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * McpCatalogModel — view model for the Armory's MCP Servers tab. Drives
 * list + create/edit/delete over the window-scoped `mcp.catalog.*` App API
 * (agentmux-srv/src/server/app_api/mcp.rs). Every row here is global by
 * construction — the catalog only ever lists/creates/edits is_global rows.
 * Per-agent private servers and bind/unbind live in the Agent-setup modal
 * (AgentMcpModel), not here.
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

export class McpCatalogModel {
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

    constructor() {
        this.selectedAtom = createMemo(() => {
            const id = this.selectedIdAtom();
            if (!id) return null;
            return this.serversAtom().find((s) => s.id === id) ?? null;
        });
        void this.refresh();
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
        // Solid signals are GC'd with the instance; nothing to unsubscribe.
    }
}
