// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Candidate accounts for the agent-pane failure row's "Bind account" action
 * — SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md §3.3.
 *
 * The inverse of `bind-to-agent-menu.ts`'s `computeBindCandidates` (one
 * account → many agents, the Armory's perspective): this is one agent → many
 * accounts, the failure row's perspective. Filtering RULES are the same
 * ones that spec reuses verbatim from
 * `SPEC_ACCOUNT_ADOPTION_PIGGYBACK_LOGIN_2026_08_09.md` §2.3 — alias
 * canonicalization on both sides, OAuth-class accounts only, exclude the
 * already-linked account, sort valid-then-recent.
 *
 * Deliberately a separate pure function rather than reusing
 * `computeBindCandidates` directly — that function's shape (candidate
 * AGENTS for one account) doesn't invert cleanly, and duplicating this much
 * smaller filter is cheaper than forcing one function to serve both
 * directions. It DOES reuse `resolveProviderAlias`, the actual thing that
 * must not drift between the two.
 */

import { resolveProviderAlias } from "@/app/view/agent/providers";
import type { Account } from "@/app/view/identity/identity-model";

/** Compute the accounts this agent's failure row could one-click bind to. */
export function computeAccountBindCandidates(
    agentProviderId: string,
    accounts: Account[],
    /** The account already linked to this agent for this provider, if any —
     *  excluded (nothing to adopt). */
    excludeAccountId?: string,
): Account[] {
    const provider = resolveProviderAlias(agentProviderId);
    return accounts
        .filter((a) => a.id !== excludeAccountId)
        // CLI-OAuth accounts only — service (api-key) accounts don't gate
        // spawns the same way and are handled by direct linking elsewhere,
        // per the spec's non-goals (§3.5).
        .filter((a) => a.secret_ref?.backend === "oauth_config_dir")
        .filter((a) => resolveProviderAlias(a.provider) === provider)
        .sort((a, b) => {
            const validA = a.status === "valid" ? 0 : 1;
            const validB = b.status === "valid" ? 0 : 1;
            if (validA !== validB) return validA - validB;
            // Most-recently-updated first among accounts with the same
            // validity bucket.
            return b.updated_at.localeCompare(a.updated_at);
        });
}
