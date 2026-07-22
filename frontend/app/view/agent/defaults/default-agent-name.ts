// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Default agent name on launch (#780) — the name field is empty when the
 * launch form opens, so the user has to type something before they can
 * click Launch. Friction with no payoff for the common case: pre-populate
 * `<Provider> Agent` (suffixed `2`, `3`, ... on collision) so the happy
 * path is click-Launch-immediately. The user can still edit the field.
 *
 * See docs/specs/SPEC_DEFAULT_AGENT_NAME.md (branch
 * agenta/spec-default-agent-name) for the original design.
 */

/**
 * Strip the catalog/provider display name down to a short brand word —
 * `getCliCatalogEntry(...).displayName` / `ProviderDefinition.displayName`
 * always carries a "Code"/"CLI"/"Code CLI" suffix ("Claude Code", "Codex
 * CLI", "Kimi Code CLI") that reads redundant once " Agent" is appended
 * ("Claude Code Agent"). Providers with no such suffix ("OpenClaw", "Pi")
 * pass through unchanged.
 */
export function cleanProviderLabel(displayName: string): string {
    const stripped = displayName.replace(/\s+(Code\s+CLI|Code|CLI)$/i, "").trim();
    return stripped || displayName;
}

/**
 * `<Provider> Agent`, suffixed with the lowest unused `2`, `3`, ... on
 * collision against `existingNames`. Gaps aren't filled — deleting
 * `Claude Agent 3` doesn't make a later `Claude Agent 3` reappear.
 */
export function defaultAgentName(providerDisplayName: string, existingNames: Set<string>): string {
    const base = `${cleanProviderLabel(providerDisplayName)} Agent`;
    if (!existingNames.has(base)) return base;
    let n = 2;
    while (existingNames.has(`${base} ${n}`)) n++;
    return `${base} ${n}`;
}
