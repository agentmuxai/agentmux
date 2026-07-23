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
 * `<Provider> Agent`, suffixed with the lowest unused `2`, `3`, ... against
 * `existingNames` at call time. `existingNames` is a live snapshot (the
 * caller's currently-launched instances), not persisted counter state, so a
 * freed name IS reused: if `Claude Agent 3` is later deleted, the next call
 * finds `3` unused again and returns `Claude Agent 3` — reagent P2 on #780,
 * correcting an earlier version of this comment that claimed the opposite.
 */
export function defaultAgentName(providerDisplayName: string, existingNames: Set<string>): string {
    const base = `${cleanProviderLabel(providerDisplayName)} Agent`;
    if (!existingNames.has(base)) return base;
    let n = 2;
    while (existingNames.has(`${base} ${n}`)) n++;
    return `${base} ${n}`;
}
