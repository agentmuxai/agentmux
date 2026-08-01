// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Mirrors `agentmux-srv/src/backend/providers.rs`'s `ALIASES` map — legacy/
 * alternate provider IDs a `db_agent_identity_links` row may still carry
 * (bundle-era migrations, older definitions) that must resolve to the same
 * canonical ID `ProviderDefinition.id` uses today. The backend's own
 * `resolver::provider_class` already canonicalizes before matching (reagent
 * P1 on #2263 — a definition/link still on an alias used to silently
 * mismatch); this frontend copy exists so `launch-flow.ts`'s account-link
 * lookup does the same, instead of a strict `===` comparison that misses any
 * agent whose link row predates the canonical ID.
 *
 * Kept in sync with the Rust table via `provider-id-aliases.test.ts` (same
 * drift-guard idiom as `pin-consistency.test.ts` for CLI version pins) —
 * update both together.
 */
const PROVIDER_ID_ALIASES: Record<string, string> = {
    "claude-code": "claude",
    claude_code: "claude",
    "codex-cli": "codex",
    "gemini-cli": "gemini",
    "qwen-code": "qwen",
    "qwen3-coder": "qwen",
    "kimi-cli": "kimi",
    kimi_code: "kimi",
    "openclaw-cli": "openclaw",
    "open-claw": "openclaw",
    "copilot-cli": "copilot",
    "github-copilot": "copilot",
    copilot_cli: "copilot",
    "mux-code": "muxcode",
    mux_code: "muxcode",
};

/** Resolve a possibly-legacy provider ID to its canonical form. Returns the
 *  input unchanged if it's already canonical or unrecognized. */
export function canonicalProviderId(id: string): string {
    return PROVIDER_ID_ALIASES[id] ?? id;
}

/**
 * Pick the linked account_id for `canonicalId` from a raw
 * ListAgentIdentitiesCommand result, matching the backend spawn resolver's
 * own precedence when a migrated agent has BOTH a canonical and a
 * legacy-alias link row for the same provider (codex P1 on PR #2377).
 *
 * `db_agent_identity_links` keys on the raw `(agent_id, provider)` pair, so
 * a canonical row ("claude") and an alias row ("claude-code") can coexist
 * for the same agent. The backend query that lists them orders by the raw
 * provider column (`identities.rs::agent_identity_list_for_agent`,
 * `ORDER BY provider`), and `inject_identity_env`'s injection loop iterates
 * that same order, `HashMap::insert`-ing each OAuth binding's config-dir env
 * var — so whichever binding is processed LAST silently overwrites the env
 * var an earlier one wrote. The real spawn therefore always ends up using
 * the LAST canonical-equivalent row in that order, not the first —
 * `Array.prototype.find` would pick the wrong one whenever both rows exist.
 *
 * Every call site that resolves "the account this agent uses for this
 * provider" (the mount-time check in launch-flow.ts, and the recovery
 * lookups in useAgentControllerStatus.ts's `existingAccountIdFor`) must use
 * this, not a raw `.find()` — codex P1 on PR #2377 (second round) caught a
 * recovery-path call site that still used the strict comparison, which
 * could report a recovery success while the next spawn silently kept using
 * a stale alias-bound directory.
 */
export function lastLinkedAccountId(
    links: Array<{ provider: string; account_id: string }>,
    canonicalId: string,
): string | undefined {
    const matches = links.filter((l) => canonicalProviderId(l.provider) === canonicalId);
    return matches.length > 0 ? matches[matches.length - 1].account_id : undefined;
}

/** Exposed for the drift-guard test only. */
export const _PROVIDER_ID_ALIASES_FOR_TEST = PROVIDER_ID_ALIASES;
