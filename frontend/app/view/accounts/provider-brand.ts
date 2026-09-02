// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * provider-brand — unify the two account namespaces in the Armory UI.
 *
 * A provider-CLI OAuth login (concept A — e.g. the Claude CLI's
 * `~/.claude/.credentials.json`, stored as an IdentityAccount with
 * `provider = "claude"`, `kind = "oauth"`) IS an authorization for a Trust
 * Center *brand* (concept B — `anthropic`). The two never met: the gallery and
 * grouping only know brand ids, so a `"claude"` account was filtered out and
 * the Anthropic tile showed nothing despite the user being logged in.
 *
 * `brandForProvider` maps a CLI-OAuth provider id to its brand so accounts
 * group under the right tile. It's **display-only** — the account's stored
 * `provider` is unchanged (the resolver still injects env keyed by the real
 * CLI id at spawn). See docs/specs/archive/SPEC_TRUST_CENTER_CLI_AUTH_BINDING_2026_06_17.md.
 */

import type { AccountProvider } from "@/app/view/identity/identity-model";

/** CLI-OAuth provider id → Armory brand. Only providers whose login
 *  authorizes a brand we show as a tile are mapped; everything else passes
 *  through unchanged. */
const CLI_PROVIDER_TO_BRAND: Record<string, AccountProvider> = {
    claude: "anthropic",
    codex: "openai",
    gemini: "google",
    copilot: "github",
};

/**
 * Normalize an account's provider to the brand it should display under.
 * Brand ids and unmapped providers pass through unchanged.
 */
export function brandForProvider(provider: string): AccountProvider {
    return CLI_PROVIDER_TO_BRAND[provider] ?? (provider as AccountProvider);
}

/** True when `provider` is a CLI-OAuth id that maps onto a different brand. */
export function isCliOAuthProvider(provider: string): boolean {
    return provider in CLI_PROVIDER_TO_BRAND;
}
