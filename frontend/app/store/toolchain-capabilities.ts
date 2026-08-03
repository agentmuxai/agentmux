// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * toolchain-capabilities — the single point of entry for "is toolchain X
 * available right now," shared by every feature that needs to know.
 *
 * Before this module, each consumer (the Toolchain diagnostics widget, the
 * create-agent-from-template modal, the container-agent launch pre-flight
 * check, …) independently decided which backend RPC answered "is Docker
 * available" and never shared or cached the result. Two of those consumers
 * disagreed — one checked the CLI binary is on PATH (`ResolveCliCommand`,
 * true even when the Docker daemon is stopped), the other checked the
 * daemon actually answers a ping (`ContainerRuntimeAvailableCommand`) — so
 * the Toolchain widget could report Docker "installed" while the
 * create-agent modal greyed out the Container option as "Docker not
 * detected," on the same machine, at the same moment. See
 * docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
 *
 * This module fixes that by being the only place that decides, per
 * `CoreTool.checkKind` (toolchain-catalog.ts), which backend primitive
 * answers the question for a given tool id, and by caching + sharing the
 * result across every consumer via a module-level store (same pattern as
 * `token-usage.ts`/`agentActivity.ts` — module-level `createStore` +
 * exported functions, no class, no context provider).
 */

import { createStore, reconcile } from "solid-js/store";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { CORE_TOOLS, cliCommandForPlatform, currentPlatform } from "@/app/view/agent/providers/toolchain-catalog";

export type CapabilityStatus = "unknown" | "checking" | "available" | "unavailable";

export interface CapabilityState {
    status: CapabilityStatus;
    version?: string;
    path?: string;
    source?: string;
    checkedAt?: number;
}

const UNKNOWN: CapabilityState = { status: "unknown" };

/**
 * Liveness probes, looked up by tool id — the one place that knows which
 * backend RPC answers "is the daemon actually up" for a `checkKind:
 * "liveness"` catalog entry. Extend this map (not the dispatch logic in
 * `ensureCapability`) when a second daemon-backed tool needs this.
 */
const LIVENESS_PROBES: Record<string, () => Promise<boolean>> = {
    docker: async () => {
        const r = await RpcApi.ContainerRuntimeAvailableCommand(TabRpcClient, { timeout: 10000 });
        return r?.available === true;
    },
};

const [capabilities, setCapabilities] = createStore<Record<string, CapabilityState>>({});
const inFlight = new Map<string, Promise<CapabilityState>>();

/** Current cached state for `id` — `{status:"unknown"}` if never probed. */
export function getCapability(id: string): CapabilityState {
    return capabilities[id] ?? UNKNOWN;
}

/** True iff the last completed probe for `id` found it available. */
export function isAvailable(id: string): boolean {
    return capabilities[id]?.status === "available";
}

async function probePath(id: string): Promise<CapabilityState> {
    const tool = CORE_TOOLS.find((t) => t.id === id);
    const cliCommand = tool ? cliCommandForPlatform(tool, currentPlatform()) : id;
    const data = {
        provider_id: id,
        cli_command: cliCommand,
        npm_package: "",
        pinned_version: "",
        windows_install_command: "",
        unix_install_command: "",
    };
    try {
        const r = await RpcApi.ResolveCliCommand(TabRpcClient, data, { timeout: 12000 });
        return {
            status: "available",
            version: r.version && r.version !== "unknown" ? r.version : undefined,
            path: r.cli_path,
            source: r.source,
            checkedAt: Date.now(),
        };
    } catch {
        return { status: "unavailable", checkedAt: Date.now() };
    }
}

async function probeLiveness(id: string): Promise<CapabilityState> {
    const probe = LIVENESS_PROBES[id];
    if (!probe) {
        console.error(`toolchain-capabilities: "${id}" declares checkKind:"liveness" but has no registered probe in LIVENESS_PROBES`);
        return { status: "unavailable", checkedAt: Date.now() };
    }
    try {
        const available = await probe();
        return { status: available ? "available" : "unavailable", checkedAt: Date.now() };
    } catch {
        return { status: "unavailable", checkedAt: Date.now() };
    }
}

/**
 * Ensures a probe for `id` has run at least once (or is currently
 * running), returning its result. Concurrent callers for the same `id`
 * share one in-flight RPC rather than each firing their own — this is
 * what lets the Toolchain widget and the create-agent modal, mounted at
 * the same time, never disagree.
 *
 * `force: true` starts a fresh probe even if a cached result already
 * exists (for explicit "Refresh" actions and just-in-time pre-flight
 * checks right before a launch, where staleness is the failure mode being
 * guarded against) — but still joins an already-in-flight probe rather
 * than starting a second redundant one.
 */
export function ensureCapability(id: string, opts?: { force?: boolean }): Promise<CapabilityState> {
    // Already running — join it regardless of `force`, rather than firing
    // a second concurrent RPC for the same id.
    const existing = inFlight.get(id);
    if (existing) return existing;

    // Non-forced call with a completed result already cached — nothing to
    // do. `force` skips this and re-probes unconditionally.
    if (!opts?.force) {
        const cached = capabilities[id];
        if (cached && cached.status !== "unknown" && cached.status !== "checking") {
            return Promise.resolve(cached);
        }
    }

    const tool = CORE_TOOLS.find((t) => t.id === id);
    if (!tool) {
        const result: CapabilityState = { status: "unavailable", checkedAt: Date.now() };
        setCapabilities(id, result);
        return Promise.resolve(result);
    }

    setCapabilities(id, (prev) => ({ ...(prev ?? UNKNOWN), status: "checking" }));
    const kind = tool.checkKind ?? "path";
    const promise = (kind === "liveness" ? probeLiveness(id) : probePath(id)).then((state) => {
        setCapabilities(id, state);
        inFlight.delete(id);
        return state;
    });
    inFlight.set(id, promise);
    return promise;
}

/** Forces a fresh probe for `id`, bypassing any cached (non-in-flight) result. */
export function refreshCapability(id: string): Promise<CapabilityState> {
    return ensureCapability(id, { force: true });
}

/**
 * Polls a capability every `intervalMs` while the caller holds the
 * returned stop function (call it from `onCleanup`). This is what lets an
 * open Toolchain widget / create-agent modal notice "the user just
 * started Docker Desktop" within a few seconds, with no manual refresh
 * and no AgentMux restart — previously impossible, since the daemon
 * connection was fixed at process boot on the backend and each frontend
 * consumer only ever probed once at mount.
 */
export function watchCapability(id: string, intervalMs = 4000): () => void {
    void ensureCapability(id);
    const handle = setInterval(() => {
        void ensureCapability(id, { force: true });
    }, intervalMs);
    return () => clearInterval(handle);
}

/**
 * Clears all cached capability state and in-flight probes. This is a
 * module-level singleton shared across the whole app (by design — that
 * sharing is the point), so tests that render multiple consumers with
 * different mocked RPC results across cases must call this in
 * `beforeEach`, or a later case will see a prior case's cached result
 * instead of re-probing.
 */
export function resetCapabilities(): void {
    setCapabilities(reconcile({}));
    inFlight.clear();
}
