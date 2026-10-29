// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shared audit-log fetch, sourced from ReactiveHandler's ring buffer
// (crates/srv/src/backend/reactive/handler.rs), for the Audit manager.

import { authHeaders } from "@/app/store/auth-headers";
import { getWebServerEndpoint } from "@/util/endpoints";

export const AUDIT_LIMIT = 50;

export interface AuditEntry {
    timestamp: number;
    source_agent?: string;
    target_agent: string;
    block_id: string;
    message_hash: string;
    message_length: number;
    success: boolean;
    error_message?: string;
    request_id: string;
    /** What kind of delivery this was when it wasn't an ordinary jekt
     *  ("held_delivered", "held_retry", "identity-mismatch",
     *  "system_note"); absent for an ordinary jekt. Mirrors
     *  AuditLogEntry.outcome (crates/srv/src/backend/reactive/types.rs). */
    outcome?: string;
    /** A caller's free-text note on a fleet or pane action (e.g. ClosePane's
     *  `reason`). */
    reason?: string;
    /** "delivery" (default) | "action" | "register" | "unregister" | "swept". "action" is a
     *  state-changing RPC call (its `outcome` is the command, `source_agent` who made it:
     *  `agent:<id>`, `operator` or `unproven`). The registration kinds are
     *  agent registration and eviction bookkeeping (issue #2694): no message
     *  moved, so the feed words them as a registration, not a delivery. */
    event_kind?: string;
    /** For a "register" event: the block_id this agent_key was previously
     *  mapped to, if registering evicted an existing mapping. */
    evicted_block?: string;
    /** For a "register" event: the agent_id this block_id was previously
     *  registered to, if registering evicted an existing mapping. */
    evicted_agent?: string;
}

/** The command an "action" row records; `undefined` for any other row. */
export const actionCommand = (entry: AuditEntry): string | undefined =>
    entry.event_kind === "action" ? entry.outcome : undefined;

export async function fetchAudit(): Promise<AuditEntry[]> {
    const resp = await fetch(
        getWebServerEndpoint() + `/agentmux/reactive/audit?limit=${AUDIT_LIMIT}`,
        { headers: authHeaders() },
    );
    if (!resp.ok) {
        throw new Error(`audit: GET /agentmux/reactive/audit → ${resp.status}`);
    }
    const data = await resp.json();
    return Array.isArray(data) ? (data as AuditEntry[]) : [];
}
