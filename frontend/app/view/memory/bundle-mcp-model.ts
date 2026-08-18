// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleMcpModel — view model for a bundle's MCP Servers section (part of
 * the Bundle editor, memory-manager.tsx). Same shape as AgentMcpModel
 * (agent-mcp-model.ts) — see its doc comment for the is_global /
 * no-check_s1 details, which apply identically here, just keyed by
 * bundle_id instead of agent_id.
 *
 * One real difference from the agent-scoped model: binding an existing
 * GLOBAL server here has no effect on a spawned agent (globals already
 * reach every agent unconditionally, bundle-bound or not — composable
 * model v2, docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md). The
 * actually-functional path is `addPrivate`, which creates a BRAND-NEW
 * private server scoped to this bundle (mcp.catalog.upsert_for_bundle) —
 * that's what makes a bundle's own tool show up for every agent bound to
 * it. Bind/unbind of existing globals is still offered for parity with the
 * agent-scoped modal and because it's harmless, just not load-bearing.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";

export class BundleMcpModel {
    readonly bundleId: string;

    private _servers = createSignal<McpServerBundleListItem[]>([]);
    serversAtom: Accessor<McpServerBundleListItem[]> = this._servers[0];
    private setServers = this._servers[1];

    private _selectedId = createSignal<string | null>(null);
    selectedIdAtom: Accessor<string | null> = this._selectedId[0];
    setSelectedId = this._selectedId[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    private _adding = createSignal<boolean>(false);
    addingAtom: Accessor<boolean> = this._adding[0];
    setAdding = this._adding[1];

    selectedAtom: Accessor<McpServerBundleListItem | null>;

    // Cross-window reactivity — a bind/unbind, add, or Armory catalog edit
    // made elsewhere refreshes this view without a manual reopen.
    private unsubChanged: () => void;

    constructor(bundleId: string) {
        this.bundleId = bundleId;
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
            const list = await RpcApi.McpCatalogListForBundleCommand(TabRpcClient, { bundle_id: this.bundleId });
            this.setServers(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load MCP servers: ${(e as Error).message ?? e}`);
        }
    }

    handleSelect(server: McpServerBundleListItem): void {
        this.setError(null);
        this.setSelectedId(server.id);
    }

    async bind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.McpCatalogBindToBundleCommand(TabRpcClient, { bundle_id: this.bundleId, mcp_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Bind failed: ${(e as Error).message ?? e}`);
        }
    }

    async unbind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.McpCatalogUnbindFromBundleCommand(TabRpcClient, { bundle_id: this.bundleId, mcp_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Unbind failed: ${(e as Error).message ?? e}`);
        }
    }

    /** Creates a NEW, PRIVATE server scoped directly to this bundle —
     *  the actually-functional "give this bundle its own tool" path. See
     *  this class's own doc comment. */
    async addPrivate(name: string, config: string): Promise<void> {
        this.setError(null);
        this.setAdding(true);
        try {
            await RpcApi.McpCatalogUpsertForBundleCommand(TabRpcClient, {
                bundle_id: this.bundleId,
                name,
                config,
            });
            await this.refresh();
        } catch (e) {
            this.setError(`Add failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setAdding(false);
        }
    }

    dispose(): void {
        this.unsubChanged();
    }
}
