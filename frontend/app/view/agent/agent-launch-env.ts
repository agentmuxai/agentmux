// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Launch-environment helpers extracted from agent-model.ts (see
 * docs/specs — modularization pass, 2026-07-23). These resolve
 * host-level paths/availability needed before spawning an agent CLI.
 * No `this`/class coupling — standalone functions the model calls into.
 */

import { getApi } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Logger } from "@/util/logger";
import { DEFAULT_RUNTIME_CONFIG, type AgentRuntimeConfig } from "./types";
import type { ProviderDefinition, ProviderModel } from "./providers/types";

/**
 * Check if Node.js is available for a provider actually installed via
 * `npm install -g <npmPackage>` (AgentInstallModal -> install.start ->
 * agentmux-srv's install_handlers.rs) — which today is every provider
 * except kimi (pip-based, `npmPackage: ""`).
 *
 * **The `claude` exemption below is a workaround for a real PATH mismatch,
 * not a claim Claude doesn't need Node.** This check's probe
 * (`getApi().checkNodejsAvailable()`) runs in the CEF host process's own
 * PATH. The actual npm install/spawn runs in the agentmux-srv sidecar,
 * whose PATH is separately reconstructed from the user's login shell
 * (`agentmux-cef/src/sidecar.rs`, `resolve_login_path`) specifically so it
 * can find Homebrew/nvm-installed Node on macOS — the host process does
 * NOT get that same enrichment. So this check can genuinely disagree with
 * reality on those setups: report "Node.js is not installed" when the
 * process that actually spawns npm can find it fine.
 *
 * This function used to hardcode `providerId === "claude"` as a skip for a
 * DIFFERENT, wrong reason (the stale belief that Claude installs via its
 * own standalone script rather than npm — it doesn't; AgentInstallModal
 * always uses `npmPackage`). An earlier revision of this fix removed the
 * skip on that basis, which fixed the wrong reasoning but reintroduced
 * this PATH-mismatch bug for Claude specifically: a previously-exempt
 * provider could now hit a false-negative launch block on affected macOS
 * setups (Codex review finding, PR #2947). Every OTHER npm-based provider
 * was already exposed to this same pre-existing PATH-mismatch bug before
 * this change (never exempted) — extending the derivation to them isn't
 * new risk, just not-yet-fixed. Claude is kept exempt here, with an
 * accurate reason this time, until `checkNodejsAvailable()` itself is
 * fixed to probe the same enriched PATH the sidecar spawns with.
 *
 * The actual reported bug this whole change addresses — a fresh machine
 * with no Node.js at all crashing on first launch — is fixed independently
 * by `catalog.ts`'s `NODE_PREREQ`/`NPM_PREREQ`, which route through
 * `resolve.prereqs`, a backend (srv) check that already uses the correct,
 * enriched PATH. This function is a secondary, launch-time check (covers
 * e.g. Node being removed between install and launch); it was never the
 * primary fix.
 */
export async function checkNodejsForProvider(provider: Pick<ProviderDefinition, "id" | "npmPackage">): Promise<string | null> {
    if (provider.id === "claude") return null; // see note above — PATH-mismatch workaround, not "doesn't need Node"
    if (!provider.npmPackage) return null; // not npm-installed (e.g. kimi, via pip)
    try {
        const status = await getApi().checkNodejsAvailable();
        if (!status.available || !status.npm_available) {
            const missing = !status.available ? "Node.js" : "npm";
            return `${missing} is not installed. Install Node.js from https://nodejs.org/ (LTS recommended).`;
        }
        return null;
    } catch (e) {
        Logger.warn("agent", "Failed to check Node.js availability", { error: String(e) });
        return null; // Don't block launch on check failure — let npm install fail with its own error
    }
}

/**
 * Return the AgentMux user-home base directory as an absolute path.
 *
 * Routed by the CEF host so per-agent paths (working dir, `GH_CONFIG_DIR`, …)
 * land in the right place for the instance type:
 *   - Portable: `<portable>/data`
 *   - Installed: `~/.agentmux`
 *   - `AGENTMUX_DATA_HOME` env override: wins over both.
 *
 * Falls back to `$HOME/.agentmux` only if the host IPC hasn't populated the
 * cached value yet (shouldn't happen in practice — `initCefApi` fetches it
 * before any agent launch).
 *
 * See `docs/specs/portable-agent-working-dirs.md`.
 */
export function agentmuxHome(): string {
    const fromHost = getApi().getUserHomeDir();
    if (fromHost) return fromHost;
    const home = getApi().getEnv("HOME") || getApi().getEnv("USERPROFILE") || "~";
    return `${home}/.agentmux`;
}

/**
 * Resolve the version-isolated CLI install directory.
 */
export function resolveCliDir(version: string, providerId: string): string {
    return `${agentmuxHome()}/instances/v${version}/cli/${providerId}`;
}

/**
 * Resolve the effective provider for a launch, preferring the agent's
 * bound ABF bundle's copy over its own (driftable) `provider` field.
 *
 * The bundle is the readonly-once-set source of truth
 * (ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7.4.1);
 * `AgentDefinition.provider` can drift post-creation via `agent.define`'s
 * `if_exists=update` path. The backend already resolves this way for
 * both `agent.open`'s own spawn path (`agent_open.rs`) and the layer-3
 * credential gate (`identity/resolver/inject.rs`) — without this, the
 * CLI binary `launchAgentDefinition` actually launches could disagree
 * with which provider's credentials the backend gate validates and
 * injects (PR #2592 review — fixing only the backend gate wasn't
 * sufficient).
 *
 * Extracted as its own function, separate from `launchAgentDefinition`,
 * so this resolution logic is unit-testable in isolation — that
 * function's own RPC/side-effect surface (Node.js checks, CLI
 * resolution, content/skill loading, instance creation, etc.) has no
 * existing test harness anywhere in this codebase (every caller mocks
 * the whole function away), so testing this piece through it isn't
 * practical.
 *
 * Falls back to `agent.provider` on any failure (unbound, fetch error,
 * empty bundle provider) — this must never block a launch on its own.
 */
export async function resolveEffectiveLaunchProvider(agent: AgentDefinition): Promise<string> {
    if (!agent.memory_id) return agent.provider;
    try {
        const bundle = await RpcApi.GetMemoryCommand(TabRpcClient, { id: agent.memory_id });
        return bundle?.provider || agent.provider;
    } catch (e: any) {
        Logger.warn("agent", "Failed to resolve agent's bound bundle for provider; falling back to agent.provider", {
            agentId: agent.id,
            error: String(e),
        });
        return agent.provider;
    }
}

/**
 * Resolve the initial `agent:runtime` block-meta value (AgentRuntimeConfig)
 * for a fresh launch. Extracted as its own pure function — same reasoning
 * as `resolveEffectiveLaunchProvider` above — so it's unit-testable without
 * `launchAgentDefinition`'s much larger RPC/side-effect surface.
 *
 * `launchAgentDefinition` previously never set `agent:runtime` on a fresh
 * launch at all, so `getRuntimeConfig`'s fallback (`DEFAULT_RUNTIME_CONFIG`,
 * hardcoded to Claude's `"sonnet"`) silently applied regardless of harness —
 * harmless for Claude agents, but a non-Claude agent's very first turn
 * could carry a model string its own provider doesn't even recognize until
 * the user opened the model picker and chose a real one.
 *
 * Precedence: an explicit `overrides.model` (e.g. a choice made in
 * AgentCreateFromTemplateModal) wins; otherwise the effective provider's
 * own `default: true` model; otherwise `DEFAULT_RUNTIME_CONFIG.model` as
 * the last-resort fallback (a provider that declares no `models` list at
 * all — e.g. one still on the raw-passthrough output format).
 */
export function resolveInitialRuntimeConfig(
    overridesModel: string | undefined,
    providerModels: ProviderModel[] | undefined,
): AgentRuntimeConfig {
    const model = overridesModel || providerModels?.find((m) => m.default)?.value || DEFAULT_RUNTIME_CONFIG.model;
    return { ...DEFAULT_RUNTIME_CONFIG, model };
}
