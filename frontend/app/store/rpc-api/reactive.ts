// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Jekt/muxbus registration status for one agent — powers the Stash
// "Registration" tab (issue #2696). Backed by the `reactive.registrations`
// WS RPC command (agentmux-srv/src/server/reactive.rs's
// register_reactive_ws_handlers), not the pre-existing `/agentmux/reactive/*`
// HTTP routes (those exist for cross-instance/LAN server-to-server
// forwarding, not frontend consumption — this file is the first frontend
// caller of anything in that domain).

import { RpcClient } from "../rpc-client";

/** Mirrors agentmux-srv's `AgentRegistration` (backend/reactive/types.rs). */
export interface ReactiveAgentRegistration {
    agent_id: string;
    block_id: string;
    tab_id?: string;
    registered_at: number;
    last_seen: number;
    registration_nonce: number;
}

/** One OTHER instance/channel on this host also claiming this agent_id —
 *  the actual risk signal the "registered elsewhere too" badge surfaces. */
export interface ReactiveRemoteRegistration {
    channel: string;
    pid: number;
    updated_at: number;
}

/** Most recent #2695 identity-mismatch audit entry for this agent, if any. */
export interface ReactiveMismatchSummary {
    timestamp: number;
    block_id: string;
    error_message?: string;
}

export interface ReactiveRegistrationsResult {
    local: ReactiveAgentRegistration | null;
    remote: ReactiveRemoteRegistration[];
    recent_mismatch: ReactiveMismatchSummary | null;
}

export const ReactiveApi = {
    GetReactiveRegistrationsCommand(
        client: RpcClient,
        data: { agent_id: string },
        opts?: RpcOpts,
    ): Promise<ReactiveRegistrationsResult> {
        return client.rpcCall("reactive.registrations", data, opts);
    },
};
