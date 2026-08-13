// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shared audit-log fetch, sourced from ReactiveHandler's ring buffer
// (agentmux-srv/src/backend/reactive/handler.rs). Imported by both the
// Audit manager (shows everything) and the Supervisor manager (shows only
// Supervisor-originated rows, filtered client-side on `outcome`).

import { getWebServerEndpoint } from "@/util/endpoints";
import { authedHeaders } from "@/app/view/warden-shared/warden-shared";

export const WARDEN_AUDIT_LIMIT = 50;

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
    /** "nudge_sent" | "nudge_failed" | "nudge_declined" — present only for
     *  Supervisor-originated entries; absent for ordinary jekt entries.
     *  "nudge_sent" only when delivery actually succeeded; "nudge_failed"
     *  when a nudge was attempted but delivery itself failed (see
     *  `success`/`error_message`). Mirrors AuditLogEntry.outcome
     *  (agentmux-srv/src/backend/reactive/types.rs). */
    outcome?: string;
    /** Supervisor's stated reasoning, populated alongside `outcome`. */
    reason?: string;
}

export async function fetchWardenAudit(): Promise<AuditEntry[]> {
    const resp = await fetch(
        getWebServerEndpoint() + `/agentmux/reactive/audit?limit=${WARDEN_AUDIT_LIMIT}`,
        { headers: authedHeaders() },
    );
    if (!resp.ok) {
        throw new Error(`warden: GET /agentmux/reactive/audit → ${resp.status}`);
    }
    const data = await resp.json();
    return Array.isArray(data) ? (data as AuditEntry[]) : [];
}
