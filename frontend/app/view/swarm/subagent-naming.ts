// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Asking for a subagent's name, and taking it in when it arrives. Shared by the
 * Swarm pane and the agent pane's activity dock, which each had their own copy.
 *
 * A subagent is normally named as soon as it is seen, from the description its
 * parent gave it (`subagent_watcher`); workflow members have none and get a
 * generated name. Expanding a row that is still unnamed asks again, in case that
 * call is still running or failed. The name comes back as a `subagent:named`
 * event to every client, not as this call's result.
 */

import { callBackendService } from "@/app/store/mos";
import { muxEventSubscribe } from "@/app/store/mps";
import type { ActiveSubagent } from "./swarm-model";

/** Requests in flight, so the dock and the Swarm pane don't ask twice at once. */
const inFlight = new Set<string>();

/** Ask the backend to name subagent `agentId`. Its spend is recorded from srv's
 *  `ambient:spent` event (store/ambient-spend.ts), not here. */
export function requestSubagentName(agentId: string): void {
    if (inFlight.has(agentId)) return;
    inFlight.add(agentId);
    void callBackendService("subagent", "GenerateName", [agentId])
        .catch(() => {
            // The row keeps its fallback label; the next expand asks again.
        })
        .finally(() => inFlight.delete(agentId));
}

/** Call `handler` with each subagent name as it arrives. Returns the unsubscribe. */
export function onSubagentNamed(handler: (agentId: string, displayName: string) => void): () => void {
    return muxEventSubscribe({
        eventType: "subagent:named",
        handler: (event: MuxEvent) => {
            const data = event?.data as { agentId?: string; displayName?: string } | undefined;
            if (data?.agentId && data.displayName) handler(data.agentId, data.displayName);
        },
    });
}

/** `list` with subagent `agentId` renamed; the other entries keep their identity. */
export function withSubagentName(list: ActiveSubagent[], agentId: string, displayName: string): ActiveSubagent[] {
    return list.map((s) => (s.agent_id === agentId ? { ...s, display_name: displayName } : s));
}
