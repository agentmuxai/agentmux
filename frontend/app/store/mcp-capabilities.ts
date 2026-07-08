// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * mcp-capabilities — "is this MCP server actually reachable right now,"
 * the MCP-server analogue of toolchain-capabilities.ts's CLI/daemon probes.
 * Same shape deliberately: module-level `createStore`, `ensureCapability`/
 * `watchCapability`, in-flight de-dup so two mounted consumers (e.g. an
 * Armory catalog row and a future per-agent status pill) never fire
 * duplicate probes or disagree with each other.
 *
 * Backed by `mcp.catalog.probe` (agentmux-srv/src/server/app_api/mcp.rs) —
 * the window-scoped, agent-independent probe, since the primary consumer is
 * the Armory's MCP Servers catalog, which has no agent context. Keyed by
 * MCP server id, not a fixed catalog of known tool ids (toolchain-catalog.ts
 * enumerates a small fixed set of CLIs; MCP servers are user-defined rows).
 *
 * See docs/specs/SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md §4.4.
 * A "connected" result means the MCP handshake succeeded — it does NOT mean
 * every prerequisite the server itself depends on is satisfied (e.g.
 * ableton-mcp's process starts and answers `initialize` whether or not
 * Ableton Live is actually running; that gap is why a catalog entry's
 * `prereq_note` static remediation text matters alongside this dynamic
 * check, not instead of it).
 */

import { createStore, reconcile } from "solid-js/store";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

const PROBE_TIMEOUT_MS = 10000;

export type McpCapabilityStatus = "unknown" | "checking" | McpProbeResult["status"];

export interface McpCapabilityState {
    status: McpCapabilityStatus;
    toolCount?: number;
    serverName?: string;
    serverVersion?: string;
    error?: string;
    checkedAt?: number;
}

const UNKNOWN: McpCapabilityState = { status: "unknown" };

const [capabilities, setCapabilities] = createStore<Record<string, McpCapabilityState>>({});
const inFlight = new Map<string, Promise<McpCapabilityState>>();

/** Current cached state for MCP server `id` — `{status:"unknown"}` if never probed. */
export function getMcpCapability(id: string): McpCapabilityState {
    return capabilities[id] ?? UNKNOWN;
}

/** True iff the last completed probe for `id` found the server reachable. */
export function isMcpConnected(id: string): boolean {
    return capabilities[id]?.status === "connected";
}

async function probeOne(id: string): Promise<McpCapabilityState> {
    try {
        const r = await RpcApi.McpCatalogProbeCommand(TabRpcClient, { id }, { timeout: PROBE_TIMEOUT_MS });
        return {
            status: r.status,
            toolCount: r.tool_count ?? undefined,
            serverName: r.server_name ?? undefined,
            serverVersion: r.server_version ?? undefined,
            error: r.error ?? undefined,
            checkedAt: Date.now(),
        };
    } catch (e) {
        return { status: "unreachable", error: (e as Error).message ?? String(e), checkedAt: Date.now() };
    }
}

/**
 * Ensures a probe for MCP server `id` has run at least once (or is
 * currently running), returning its result. Concurrent callers for the
 * same `id` share one in-flight RPC. `force: true` starts a fresh probe
 * even if a cached result already exists, but still joins an
 * already-in-flight probe rather than starting a second redundant one.
 */
export function ensureMcpCapability(id: string, opts?: { force?: boolean }): Promise<McpCapabilityState> {
    const existing = inFlight.get(id);
    if (existing) return existing;

    if (!opts?.force) {
        const cached = capabilities[id];
        if (cached && cached.status !== "unknown" && cached.status !== "checking") {
            return Promise.resolve(cached);
        }
    }

    setCapabilities(id, (prev) => ({ ...(prev ?? UNKNOWN), status: "checking" }));
    const promise = probeOne(id).then((state) => {
        setCapabilities(id, state);
        inFlight.delete(id);
        return state;
    });
    inFlight.set(id, promise);
    return promise;
}

/** Forces a fresh probe for `id`, bypassing any cached (non-in-flight) result. */
export function refreshMcpCapability(id: string): Promise<McpCapabilityState> {
    return ensureMcpCapability(id, { force: true });
}

/**
 * Polls a capability every `intervalMs` while the caller holds the
 * returned stop function (call it from `onCleanup`) — lets an open Armory
 * MCP Servers tab notice "the user just started Ableton Live" within a
 * few seconds, same pattern as toolchain-capabilities.ts's Docker liveness
 * polling.
 */
export function watchMcpCapability(id: string, intervalMs = 4000): () => void {
    void ensureMcpCapability(id);
    const handle = setInterval(() => {
        void ensureMcpCapability(id, { force: true });
    }, intervalMs);
    return () => clearInterval(handle);
}

/**
 * Clears all cached capability state and in-flight probes. Module-level
 * singleton shared across the whole app — tests that render multiple
 * consumers with different mocked RPC results across cases must call this
 * in `beforeEach`, or a later case will see a prior case's cached result.
 */
export function resetMcpCapabilities(): void {
    setCapabilities(reconcile({}));
    inFlight.clear();
}
