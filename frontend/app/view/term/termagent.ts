// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Reactive agent registration helpers.
// Extracted from termwrap.ts — standalone async functions, no TermWrap dependency.

import { getApi } from "@/store/global";
import { getWebServerEndpoint } from "@/util/endpoints";
import { fireAndForget } from "@/util/util";

// Track registered agent IDs per block to detect changes
export const registeredAgentsByBlock = new Map<string, string>();

// Build the `X-AuthKey` header expected by the sidecar's auth middleware.
// Returns an empty object if no key is available (early-boot / tests);
// callers will see a 401 in that case, which is a clearer signal than
// silently succeeding.
function authHeaders(): Record<string, string> {
    const k = getApi()?.getAuthKey?.();
    return k ? { "X-AuthKey": k } : {};
}

export async function registerAgent(agentId: string, blockId: string, tabId?: string): Promise<void> {
    try {
        const url = getWebServerEndpoint() + "/agentmux/reactive/register";
        const response = await fetch(url, {
            method: "POST",
            headers: { "Content-Type": "application/json", ...authHeaders() },
            body: JSON.stringify({
                agent_id: agentId,
                block_id: blockId,
                tab_id: tabId || "",
            }),
        });
        if (!response.ok) {
            let errorMsg = `HTTP ${response.status}`;
            try {
                const data = await response.json();
                errorMsg = data.error || errorMsg;
            } catch {
                // Response body not JSON, use status
            }
            console.error("[reactive] failed to register agent:", errorMsg);
        } else {
            console.log("[reactive] registered agent", agentId, "->", blockId);
        }
    } catch (e) {
        console.error("[reactive] error registering agent:", e);
    }
}

export async function unregisterAgent(agentId: string): Promise<void> {
    try {
        const url = getWebServerEndpoint() + "/agentmux/reactive/unregister";
        const response = await fetch(url, {
            method: "POST",
            headers: { "Content-Type": "application/json", ...authHeaders() },
            body: JSON.stringify({ agent_id: agentId }),
        });
        if (!response.ok) {
            let errorMsg = `HTTP ${response.status}`;
            try {
                const data = await response.json();
                errorMsg = data.error || errorMsg;
            } catch {
                // Response body not JSON, use status
            }
            console.error("[reactive] failed to unregister agent:", errorMsg);
        } else {
            console.log("[reactive] unregistered agent", agentId);
        }
    } catch (e) {
        console.error("[reactive] error unregistering agent:", e);
    }
}

export function handleAgentIdChange(blockId: string, newAgentId: string | undefined, tabId?: string): void {
    const previousAgentId = registeredAgentsByBlock.get(blockId);

    if (previousAgentId === newAgentId) {
        return;
    }

    if (previousAgentId) {
        fireAndForget(() => unregisterAgent(previousAgentId));
        registeredAgentsByBlock.delete(blockId);
    }

    if (newAgentId) {
        registeredAgentsByBlock.set(blockId, newAgentId);
        fireAndForget(() => registerAgent(newAgentId, blockId, tabId));
    }
}
