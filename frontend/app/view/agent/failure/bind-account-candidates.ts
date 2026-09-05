// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Candidate accounts for the agent-pane failure row's "Bind account" action
 * — SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md §3.3.
 *
 * The inverse of `bind-to-agent-menu.ts`'s `computeBindCandidates` (one
 * account → many agents, the Armory's perspective): this is one agent → many
 * accounts, the failure row's perspective. Filtering RULES mostly follow
 * `SPEC_ACCOUNT_ADOPTION_PIGGYBACK_LOGIN_2026_08_09.md` §2.3 — alias
 * canonicalization on both sides, OAuth-class accounts only, exclude the
 * already-linked account — with one deliberate departure: `status === "valid"`
 * only, not "expired selectable but marked" (see below).
 *
 * Deliberately a separate pure function rather than reusing
 * `computeBindCandidates` directly — that function's shape (candidate
 * AGENTS for one account) doesn't invert cleanly, and duplicating this much
 * smaller filter is cheaper than forcing one function to serve both
 * directions. It DOES reuse `resolveProviderAlias`, the actual thing that
 * must not drift between the two.
 *
 * **Why valid-only (amended 2026-09-05, reagentx P1):** the original design
 * allowed an expired candidate through ("adopting then re-logging is still
 * fewer steps"), visibly marked. That marking was never actually built in
 * `failure-accessory.ts` (no status-dot concept exists on `PaneRowAction`),
 * and independent of that gap, `useAgentControllerStatus.recheckAuthAfterBind`
 * (the auto-unblock check that fires right after any bind) trusts
 * `CheckCliAuthCommand`'s `authenticated` flag — which this same codebase
 * already documented as capable of an "expired-but-present false positive"
 * (see `resolveCliForRecovery`'s doc comment,
 * `retro-agent-auth-relogin-noop-2026-07-01` H2; it's *why* `relogin()`
 * never trusts that check for its own success). Offering a KNOWN-expired
 * account as a one-click "fix" then risks `declareAuthHealthy()` clearing
 * the failure row on a false positive, even though the next real turn still
 * fails. Restricting to `status === "valid"` removes this self-inflicted
 * case entirely, at the cost of the (unimplemented) "expired but still
 * useful" convenience — losing a convenience is a far smaller cost than a
 * row that silently lies about being fixed.
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
        // Valid only — see this module's doc comment for why an expired
        // candidate is excluded entirely rather than offered-but-marked.
        .filter((a) => a.status === "valid")
        .sort((a, b) => b.updated_at.localeCompare(a.updated_at));
}
