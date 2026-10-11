// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What Connectors → MCP servers shows under "In use now": every MCP server a
 * running agent's CLI reports having, from every source, with each agent's
 * status (`mcp.observed.list`, crates/srv/src/backend/mcp_observed.rs).
 * SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md §4.
 */

import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { addWSReconnectHandler } from "@/app/store/ws";
import type { McpObservedAgent } from "@/types/rpc/McpObservedAgent";
import { createSignal, type Accessor } from "solid-js";

/** One agent's view of a server. */
export interface ServerUse {
    blockId: string;
    agent: string;
    status: string;
    tools: number;
}

/** A server, with every running agent that reports it. */
export interface ObservedServerRow {
    name: string;
    source: string;
    uses: ServerUse[];
}

/** Where a server comes from, in a few words. */
export function sourceLabel(source: string): string {
    switch (source) {
        case "builtin":
            return "Built in";
        case "catalog":
            return "Catalog";
        case "claudeai":
            return "claude.ai connector";
        case "user":
            return "Account config";
        case "project":
            return "Project .mcp.json";
        case "local":
            return "Local config";
        case "dynamic":
            return "Added at launch";
        case "managed":
        case "enterprise":
            return "Managed";
        default:
            return source || "Unknown";
    }
}

const SOURCE_ORDER = ["builtin", "catalog", "claudeai", "user", "project", "local", "dynamic", "managed", "enterprise"];

function sourceRank(source: string): number {
    const i = SOURCE_ORDER.indexOf(source);
    return i < 0 ? SOURCE_ORDER.length : i;
}

/** One row per server, AgentMux's own first, then by source and name; each
 *  row's agents by name. A server two agents get from different places is
 *  one row per source. */
export function groupObserved(agents: McpObservedAgent[]): ObservedServerRow[] {
    const rows = new Map<string, ObservedServerRow>();
    for (const agent of agents) {
        for (const s of agent.servers) {
            const key = `${s.source}\u0000${s.name}`;
            let row = rows.get(key);
            if (!row) {
                row = { name: s.name, source: s.source, uses: [] };
                rows.set(key, row);
            }
            row.uses.push({ blockId: agent.block_id, agent: agent.label, status: s.status, tools: s.tools });
        }
    }
    const list = [...rows.values()];
    for (const row of list) row.uses.sort((a, b) => a.agent.localeCompare(b.agent));
    list.sort((a, b) => sourceRank(a.source) - sourceRank(b.source) || a.name.localeCompare(b.name));
    return list;
}

/** A row's state in a few words: all connected, or how many aren't. */
export function rowSummary(row: ObservedServerRow): string {
    const failing = row.uses.filter((u) => u.status !== "connected").length;
    const agents = `${row.uses.length} ${row.uses.length === 1 ? "agent" : "agents"}`;
    return failing === 0 ? agents : `${agents}, ${failing} not connected`;
}

/** Every open model, refreshed when the connection to srv comes back: srv's
 *  reports live in its memory, so a restarted srv has none, and can't send
 *  an event for the ones it lost. `addWSReconnectHandler` has no
 *  unregister, so one handler serves them all. */
const openModels = new Set<McpObservedModel>();
let reconnectHandlerAdded = false;

function refreshOnReconnect(model: McpObservedModel): void {
    openModels.add(model);
    if (reconnectHandlerAdded) return;
    reconnectHandlerAdded = true;
    addWSReconnectHandler(() => {
        for (const m of openModels) void m.refresh();
    });
}

export class McpObservedModel {
    private _rows = createSignal<ObservedServerRow[]>([]);
    rowsAtom: Accessor<ObservedServerRow[]> = this._rows[0];
    private setRows = this._rows[1];

    private _loaded = createSignal(false);
    loadedAtom: Accessor<boolean> = this._loaded[0];

    private unsubs: (() => void)[];
    /** The latest refresh: an older reply arriving after it is dropped. */
    private request = 0;

    constructor() {
        refreshOnReconnect(this);
        void this.refresh();
        // A report changed, or the catalog did (a server's source can become
        // "catalog").
        this.unsubs = [WpsEvent.McpObserved, WpsEvent.McpChanged].map((eventType) =>
            muxEventSubscribe({ eventType, handler: () => void this.refresh() })
        );
    }

    async refresh(): Promise<void> {
        const request = ++this.request;
        try {
            const agents = await RpcApi.McpObservedListCommand(TabRpcClient);
            if (request !== this.request) return;
            this.setRows(groupObserved(agents));
        } catch (e) {
            if (request !== this.request) return;
            // An srv without the command (an older one, perhaps after a
            // reconnect) has no reports: what is shown belongs to another
            // srv. Any other failure says nothing about the servers, so
            // what is shown stays.
            if (String((e as Error)?.message ?? e).includes("unknown command")) this.setRows([]);
        }
        this._loaded[1](true);
    }

    dispose(): void {
        openModels.delete(this);
        for (const unsub of this.unsubs) unsub();
    }
}
