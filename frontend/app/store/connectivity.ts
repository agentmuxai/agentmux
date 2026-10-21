// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether each service AgentMux depends on can be reached, as srv tracks it
 * (`crates/srv/src/connectivity`): `connectivity.status` when a window opens,
 * kept live by the `connectivity:changed` event. The status bar's Offline
 * indicator reads it, and the MuxBus indicator defers to it.
 * docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md §4.1.
 */

import { createSignal } from "solid-js";

import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { clockTime } from "@/store/muxbus-delivery";
import type { ConnectivityStatus } from "@/types/rpc/ConnectivityStatus";
import type { ServiceStatus } from "@/types/rpc/ServiceStatus";

export type { ConnectivityStatus } from "@/types/rpc/ConnectivityStatus";
export type { ServiceStatus } from "@/types/rpc/ServiceStatus";

/** The `connectivity:changed` WebSocket event. */
export const CONNECTIVITY_EVENT = WpsEvent.ConnectivityChanged;

/**
 * How long a service stays degraded before the status bar says so: srv
 * clears a single failed request within one probe, and that is not worth
 * a glance. Offline shows at once.
 */
export const DEGRADED_QUIET_MS = 30_000;

const INTERNET = "internet";
const MUXBUS = "muxbus";
const STATES = new Set(["online", "degraded", "offline", "unknown"]);

const [connectivity, setConnectivity] = createSignal<ConnectivityStatus | null>(null);

/** What srv last said, or null before it has. */
export { connectivity };

/**
 * Take a status from `connectivity.status` or the event; ignores anything
 * malformed, and anything older than what is shown (the two can arrive out
 * of order).
 */
export function acceptConnectivity(next: unknown): void {
    const s = next as Partial<ConnectivityStatus> | null | undefined;
    if (typeof s?.revision !== "number" || !Array.isArray(s.services)) return;
    const wellFormed = s.services.every(
        (svc) => typeof svc?.service === "string" && STATES.has(svc.state) && typeof svc.since_ms === "number"
    );
    if (!wellFormed) return;
    const current = connectivity();
    if (current && current.revision >= s.revision) return;
    setConnectivity(s as ConnectivityStatus);
}

/** Ask srv for the status now (a window opened after the last event). */
export async function loadConnectivity(): Promise<void> {
    try {
        acceptConnectivity(await RpcApi.ConnectivityStatusCommand(TabRpcClient));
    } catch {
        // An older srv without the command: nothing to show.
    }
}

export interface ConnectivityIndicator {
    tone: "warn" | "error";
    label: string;
    tip: string;
    /** The services it names, by id (`muxbus`, `model_api:anthropic`). */
    services: string[];
}

/** "the Anthropic API", "MuxBus". */
function displayName(s: ServiceStatus): string {
    return s.service.startsWith("model_api:") ? `the ${s.label}` : s.label;
}

/** "the Anthropic API since 03:12; MuxBus since 03:14". */
function sinceList(services: ServiceStatus[]): string {
    return services.map((s) => `${displayName(s)} since ${clockTime(s.since_ms)}`).join("; ");
}

/**
 * What the status bar shows, or null when everything answers: "Offline" when
 * any service can't be reached at all, "Connection issues" when some of its
 * requests fail for longer than {@link DEGRADED_QUIET_MS}. The tooltip names
 * each service and since when.
 */
export function connectivityIndicator(
    s: ConnectivityStatus | null | undefined,
    nowMs: number
): ConnectivityIndicator | null {
    if (!s) return null;
    const internet = s.services.find((svc) => svc.service === INTERNET);
    const others = s.services.filter((svc) => svc.service !== INTERNET);
    const offline = others.filter((svc) => svc.state === "offline");
    const degraded = others.filter((svc) => svc.state === "degraded" && nowMs - svc.since_ms >= DEGRADED_QUIET_MS);
    if (offline.length === 0 && degraded.length === 0) return null;
    const parts: string[] = [];
    if (internet?.state === "offline") parts.push(`No internet connection since ${clockTime(internet.since_ms)}.`);
    if (offline.length > 0) parts.push(`Can't reach ${sinceList(offline)}.`);
    if (degraded.length > 0) parts.push(`Some requests to ${sinceList(degraded)} are failing.`);
    parts.push("AgentMux keeps checking and clears this when they answer.");
    return {
        tone: offline.length > 0 ? "error" : "warn",
        label: offline.length > 0 ? "Offline" : "Connection issues",
        tip: parts.join(" "),
        services: [...offline, ...degraded].map((svc) => svc.service),
    };
}

/**
 * Whether the Offline indicator already says MuxBus can't be reached: the
 * MuxBus indicator then leaves its "reconnecting" out, so one problem shows
 * once. Its "sign in" still shows: that one is an action, not reachability.
 */
export function connectivityCoversMuxbus(s: ConnectivityStatus | null | undefined, nowMs: number): boolean {
    return connectivityIndicator(s, nowMs)?.services.includes(MUXBUS) ?? false;
}
