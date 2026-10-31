// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the pane does once the agent is connected: on a first connect (no existing session) the
 * startup sequence is sent as the opening turn (sendStartupSequence.ts). Built before the launch
 * flow that calls it; `send` is read only when it runs, so it can come from a hook created later.
 */

import { getApi } from "@/app/store/global";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";
import { RpcApi, type AgentDefinition } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { loadAccounts } from "@/app/view/identity/identity-model";
import type { LogFn } from "../types";
import { sendStartupSequence } from "./sendStartupSequence";

export function startupOnReady(opts: {
    blockId: string;
    agentId: string;
    block: () => { meta?: Record<string, unknown> } | null | undefined;
    currentAgent: () => AgentDefinition | undefined;
    peerAgents: () => AgentDefinition[];
    providerDisplayName: () => string;
    send: (payload: string) => Promise<void>;
    log: LogFn;
}): () => void {
    return () =>
        void sendStartupSequence({
            sessionId: () => opts.block()?.meta?.["agent:sessionid"] as string | undefined,
            agent: opts.currentAgent,
            providerDisplayName: opts.providerDisplayName,
            workDir: () => (opts.block()?.meta?.["cmd:cwd"] as string | undefined) ?? "",
            version: () => getApi().getAboutModalDetails().version,
            peerAgents: opts.peerAgents,
            getAgentContent: (contentType) => RpcApi.GetAgentContentCommand(TabRpcClient, { agent_id: opts.agentId, content_type: contentType }),
            listIdentities: () => RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: opts.agentId }),
            loadAccounts,
            send: opts.send,
            isBusy: () => workingFromPhase(paneSnapshot(opts.blockId)?.turnPhase ?? { kind: "Idle" }),
            log: opts.log,
        });
}
