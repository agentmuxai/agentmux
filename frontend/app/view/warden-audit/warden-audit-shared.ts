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
    /** "delivery" (default, the only kind this feed shows) | "register" |
     *  "unregister" — the latter two are agent registration/eviction
     *  bookkeeping (issue #2694), not a jekt delivery or Supervisor
     *  decision, and are filtered out of this feed by `fetchWardenAudit`
     *  below rather than rendered as misleading blank "ok" rows. Surfaced
     *  in the Stash "Registration" tab instead (issue #2696), the
     *  purpose-built surface for registration/eviction history. */
    event_kind?: string;
    /** For a "register" event: the block_id this agent_key was previously
     *  mapped to, if registering evicted an existing mapping. Not rendered
     *  in this feed — see `event_kind`'s doc comment. */
    evicted_block?: string;
    /** For a "register" event: the agent_id this block_id was previously
     *  registered to, if registering evicted an existing mapping. Not
     *  rendered in this feed — see `event_kind`'s doc comment. */
    evicted_agent?: string;
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
    const entries = Array.isArray(data) ? (data as AuditEntry[]) : [];
    // register/unregister entries (issue #2694) are registration
    // bookkeeping, not a jekt delivery or Supervisor decision — this feed's
    // rows assume an outcome-less entry is an ordinary delivery (message
    // bytes, success/fail), so an unfiltered register/unregister row would
    // render as a misleading blank "ok, 0b" line. `undefined` passes
    // through too: back-compat with any entry predating this field, which
    // is always a delivery.
    return entries.filter((e) => e.event_kind === undefined || e.event_kind === "delivery");
}
