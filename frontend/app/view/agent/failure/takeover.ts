// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * takeover — ask the srv to take this pane's agent over from the other
 * AgentMux instance on this host that is running it
 * (`POST /api/v1/agent/takeover`). The srv finds that instance, asks it to
 * let go, and answers once the agent is free here.
 *
 * Spec: docs/specs/SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md §4.6.
 */

import { getApi } from "@/store/global";
import { getWebServerEndpoint } from "@/util/endpoints";

export interface TakeoverResult {
    /** False when no other instance was running the agent any more. */
    released: boolean;
    fromChannel?: string;
}

/**
 * Take the agent in `blockId` over. Resolves once it is free here; rejects
 * with the srv's own user-facing reason (too old to hand over, unreachable,
 * did not let go in time).
 *
 * `fetchImpl` / `endpoint` / `authKey` are injectable for tests.
 */
export async function requestAgentTakeover(
    blockId: string,
    deps: { fetchImpl?: typeof fetch; endpoint?: string; authKey?: string } = {},
): Promise<TakeoverResult> {
    const fetchImpl = deps.fetchImpl ?? fetch;
    const endpoint = deps.endpoint ?? getWebServerEndpoint();
    const authKey = deps.authKey ?? getApi()?.getAuthKey?.();
    const resp = await fetchImpl(`${endpoint}/api/v1/agent/takeover`, {
        method: "POST",
        headers: { "Content-Type": "application/json", ...(authKey ? { "X-AuthKey": authKey } : {}) },
        body: JSON.stringify({ block_id: blockId }),
    });
    let data: any = {};
    try {
        data = await resp.json();
    } catch {
        // Non-JSON body: fall through to the status-based message.
    }
    if (!resp.ok) {
        throw new Error(data?.error || `Take over failed (HTTP ${resp.status})`);
    }
    return { released: !!data?.released, fromChannel: data?.from_channel };
}
