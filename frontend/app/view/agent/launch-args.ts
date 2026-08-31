// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which launch args a pane spawns with — the one rule that decides whether an
 * agent gets `persistentLaunchArgs` or plain `launchArgs`.
 *
 * ## Why this is its own function
 *
 * It was previously inlined as `provider.controllerType === "persistent"`,
 * which is wrong for container agents and had been silently breaking every one
 * of them since the feature shipped (verified live, 2026-08-31 — no container
 * agent had ever started on the dev machine).
 *
 * A container agent runs one `docker exec` per turn: a subprocess-shaped
 * lifecycle, whatever the provider's own default controller is. Claude's
 * `persistentLaunchArgs` carry **`--input-format stream-json`**, which tells
 * the CLI that every stdin line is a JSON envelope. That contract is only
 * honoured by the persistent controller, which owns a long-lived stdin and
 * writes real envelopes. The container path (`container_spawn.rs`) writes the
 * raw message text instead, so the CLI meets the startup markdown as its first
 * line and dies immediately:
 *
 *     Error parsing streaming input line: # Session Context:
 *     SyntaxError: JSON Parse error: Unrecognized token '#'
 *
 * srv's own copy of this rule (`agent_open.rs`) already applies the container
 * override *before* deriving `is_persistent`. This is the same rule for the
 * path the UI actually launches through — the two had drifted, and only the
 * srv one was correct.
 */

/** The subset of a provider definition this decision needs. */
export interface LaunchArgsProvider {
    controllerType?: string;
    launchArgs: string[];
    persistentLaunchArgs?: string[];
}

/**
 * True when this pane should launch with the persistent controller's args.
 *
 * `agentMode === "container"` forces `false` regardless of the provider —
 * see the module doc for why this is fatal rather than cosmetic.
 */
export function isPersistentLaunch(provider: LaunchArgsProvider, agentMode: string | undefined): boolean {
    return provider.controllerType === "persistent" && agentMode !== "container";
}

/**
 * The args to launch with. Falls back to `launchArgs` whenever the provider
 * declares no persistent variant, matching the previous inline behaviour.
 */
export function selectLaunchArgs(provider: LaunchArgsProvider, agentMode: string | undefined): string[] {
    return isPersistentLaunch(provider, agentMode) && provider.persistentLaunchArgs
        ? [...provider.persistentLaunchArgs]
        : [...provider.launchArgs];
}
