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

/** Exposed for the drift-guard test only. */
export const _PROVIDER_ID_ALIASES_FOR_TEST = PROVIDER_ID_ALIASES;
