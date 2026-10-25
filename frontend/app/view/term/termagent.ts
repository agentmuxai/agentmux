// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Reactive agent registration helpers.
// Extracted from termwrap.ts — standalone async functions, no TermWrap dependency.

import { authHeaders } from "@/app/store/auth-headers";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWebServerEndpoint } from "@/util/endpoints";
import { fireAndForget } from "@/util/util";

// Track registered agent IDs per block to detect changes
export const registeredAgentsByBlock = new Map<string, string>();

async function registerAgent(agentId: string, blockId: string, tabId?: string): Promise<void> {
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

// `blockId` is optional on the wire but always sent from here: since identity
// M2 (SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §4.4.4 Q7) a name
// can be held by several live panes, and the server refuses to tear down an
// ambiguous name without a block id rather than guess. Sent over the window's
// own socket (`agent.pane.unregister`): srv answers only a connection the host
// has proven is the AgentMux window.
export async function unregisterAgent(agentId: string, blockId?: string): Promise<void> {
    try {
        await RpcApi.AgentPaneUnregisterCommand(TabRpcClient, { agent_id: agentId, block_id: blockId ?? "" });
        console.log("[reactive] unregistered agent", agentId);
    } catch (e) {
        console.error("[reactive] failed to unregister agent:", e);
    }
}

export function handleAgentIdChange(blockId: string, newAgentId: string | undefined, tabId?: string): void {
    const previousAgentId = registeredAgentsByBlock.get(blockId);

    if (previousAgentId === newAgentId) {
        return;
    }

    // Bookkeeping is synchronous; the HTTP calls are ONE sequenced task.
    // Since identity M2 the unregister is block-scoped, so the two calls no
    // longer commute: if a rename's register were processed before its
    // unregister, `unregister_block` would wipe the registration just
    // created for this pane (ReAgent P1 on PR #3560). Awaiting the
    // unregister before registering removes the race; a failed unregister
    // must not block the (re-)registration, so it is caught, not chained.
    if (previousAgentId) {
        registeredAgentsByBlock.delete(blockId);
    }
    if (newAgentId) {
        registeredAgentsByBlock.set(blockId, newAgentId);
    }
    fireAndForget(async () => {
        if (previousAgentId) {
            await unregisterAgent(previousAgentId, blockId).catch(() => {});
        }
        if (newAgentId) {
            await registerAgent(newAgentId, blockId, tabId);
        }
    });
}
