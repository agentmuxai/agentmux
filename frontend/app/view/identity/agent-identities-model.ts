// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { Account } from "./identity-model";

export interface AgentIdentityRow {
    provider: string;
    accountId: string;
    account: Account | null;
}

/**
 * Join one agent's direct links (`db_agent_identity_links`) against the
 * account cache. Pure — no RPC, no signals — so it's cheap to call from a
 * memo on every rail-selection change. `account` is `null` when the linked
 * account id has since been deleted (orphaned link, not an error case —
 * the row still renders with a "—" fallback).
 */
export function joinAgentIdentityRows(
    agentId: string,
    allLinks: AgentDefinitionIdentity[],
    accountsById: Map<string, Account>,
): AgentIdentityRow[] {
    return allLinks
        .filter((link) => link.agent_id === agentId)
        .map((link) => ({
            provider: link.provider,
            accountId: link.account_id,
            account: accountsById.get(link.account_id) ?? null,
        }));
}
