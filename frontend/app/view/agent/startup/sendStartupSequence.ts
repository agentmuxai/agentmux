// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 4).

import type { AgentDefinition } from "@/app/store/rpc-api";
import type { AgentAccounts } from "@/app/view/identity/identity-model";
import { buildStartupPayload, resolveAccounts } from "./buildStartupPayload";

/**
 * Everything the startup sequence reads or does, injected so it runs without a
 * render harness. Getters are read at the same moments the inline version read
 * them: the session and agent up front, the rest after the lookups resolve.
 */
export interface StartupSequenceDeps {
    /** `agent:sessionid` from block meta; set for a resumed session. */
    sessionId: () => string | undefined;
    agent: () => AgentDefinition | undefined;
    providerDisplayName: () => string;
    workDir: () => string;
    version: () => string;
    peerAgents: () => AgentDefinition[];
    getAgentContent: (contentType: "startup") => Promise<{ content?: string } | null>;
    listIdentities: () => Promise<Array<{ provider: string; account_id: string }>>;
    loadAccounts: () => Parameters<typeof resolveAccounts>[1];
    send: (payload: string) => Promise<void>;
    log: (tag: string, text: string, level?: "info" | "error" | "warn") => void;
}

/**
 * On first connect (no existing session), assemble a structured startup
 * payload from agent-definition + Identity data and send it as the opening
 * turn. See docs/specs/SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16.md.
 */
export async function sendStartupSequence(d: StartupSequenceDeps): Promise<void> {
    // Skip if this is a resumed session
    if (d.sessionId()) return;

    try {
        const agent = d.agent();
        if (!agent) return;

        // Gather inputs in parallel where possible
        const [startupContentResult, version, identityLinks] = await Promise.all([
            d.getAgentContent("startup").catch(() => null),
            Promise.resolve(d.version()),
            d.listIdentities().catch(() => []),
        ]);

        // A bundle's instructions are no longer sent here: the agent's Bundles
        // list goes into its startup file at launch
        // (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md §3.6). The freeform
        // "startup" blob still is, which keeps seed-manifest content.
        const startupContent = startupContentResult?.content ?? null;

        // Resolve assigned accounts from the same db_agent_identity_links
        // rows spawn-time credential resolution and the agent pane's own
        // Identity tab already use — NOT the legacy AgentDefinition.accounts
        // JSON blob, which can silently diverge from what the agent
        // actually launches with (see docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md §1).
        const agentAccounts: AgentAccounts = {};
        for (const link of identityLinks) {
            agentAccounts[link.provider as keyof AgentAccounts] = link.account_id;
        }
        const accounts = resolveAccounts(agentAccounts, d.loadAccounts());

        const payload = buildStartupPayload({
            agent,
            providerDisplayName: d.providerDisplayName(),
            workDir: d.workDir(),
            version,
            accounts,
            peerAgents: d.peerAgents(),
            startupContent,
        });

        if (payload) {
            d.log("agent", "sending startup sequence");
            await d.send(payload);
        }
    } catch (err) {
        d.log("warn", `startup sequence failed: ${err}`, "warn");
    }
}
