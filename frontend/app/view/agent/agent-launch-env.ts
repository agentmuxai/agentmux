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
import type { AgentDefinition } from "@/app/store/rpc-api";

/**
 * Check that Node.js and npm are available for a provider installed via
 * `npm install -g <npmPackage>` (AgentInstallModal -> install.start ->
 * agentmux-srv's install_handlers.rs) — every provider except kimi
 * (pip-based, `npmPackage: ""`).
 *
 * Asks srv (`resolve.prereqs`), the process that actually runs npm, with the
 * PATH it runs npm with (reconstructed from the user's login shell, so
 * Homebrew/nvm installs on macOS are found). This used to probe the CEF
 * host's own PATH (`checkNodejsAvailable`), which lacks that enrichment and
 * could report "Node.js is not installed" when npm would work; Claude was
 * exempted to dodge that false negative (Codex P1, PR #2947). Probing where
 * npm runs removes the mismatch, and with it the exemption.
 *
 * `catalog.ts`'s `NODE_PREREQ`/`NPM_PREREQ` gate installs through the same
 * command; this is the launch-time check (e.g. Node removed after install).
 */
export async function checkNodejsForProvider(provider: Pick<ProviderDefinition, "id" | "npmPackage">): Promise<string | null> {
    if (!provider.npmPackage) return null; // not npm-installed (e.g. kimi, via pip)
    try {
        const { results } = await RpcApi.ResolvePrereqsCommand(TabRpcClient, { tools: ["node", "npm"] });
        const found = (tool: string) => results?.find((r) => r.tool === tool)?.found ?? false;
        if (!found("node") || !found("npm")) {
            const missing = !found("node") ? "Node.js" : "npm";
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
 * Routed by the CEF host so per-agent paths (e.g. the working dir)
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
 * The CLI binary a pane's `cmd` meta should name, as the backend resolves it
 * (`ResolveCli`, the same call `flows/launch-flow.ts` makes). The backend
 * installs the CLI if it's missing.
 *
 * This used to be built here as
 * `${agentmuxHome()}/instances/v<version>/cli/<provider>/node_modules/.bin/<cli>`.
 * Since v0.56.7 the backend installs CLIs under the channel's own data dir
 * (`DataPaths::from_env()`), which `agentmuxHome()` (the host's global
 * `~/.agentmux`) is not, and the string also lacked Windows' `.cmd`. The
 * backend spawns whatever `cmd` says, so a pane relaunched through this path
 * failed every spawn with "The system cannot find the path specified" (Agent3
 * on 0.57.0, 2026-09-24) while panes seeded by the backend kept working.
 */
export async function resolveCliBin(provider: ProviderDefinition, blockId: string): Promise<string> {
    const result = await RpcApi.ResolveCliCommand(
        TabRpcClient,
        {
            provider_id: provider.id,
            cli_command: provider.cliCommand,
            npm_package: provider.npmPackage,
            pinned_version: provider.pinnedVersion,
            windows_install_command: provider.windowsInstallCommand,
            unix_install_command: provider.unixInstallCommand,
            block_id: blockId,
        },
        // Same budget as launch-flow.ts: a first launch may npm-install.
        { timeout: 300000 },
    );
    if (!result?.cli_path) {
        throw new Error(`ResolveCli returned no CLI path for provider '${provider.id}'`);
    }
    return result.cli_path;
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
        const bundle = await RpcApi.GetBundleCommand(TabRpcClient, { id: agent.memory_id });
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

/**
 * Commit a launch to its block — identity M4b-3
 * (docs/specs/SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.8).
 *
 * Order matters, so it lives here, testable apart from
 * `launchAgentDefinition`:
 * 1. Record the launch row (`CreateAgentInstanceCommand`).
 * 2. One `SetMeta` carrying the block meta **and** the row's id as
 *    `agentInstanceId` — in the same write as `agentId`, because setting
 *    `agentId` mounts the agent view, whose own launch flow resyncs. `null`
 *    when no row was recorded, so a stale stamp from the pane's previous
 *    launch never survives.
 * 3. Resync the controller — where a continuation eager-resumes, now bound.
 *
 * Recorded after the resync (as before M4b-3), a continuation resumed before
 * its row was bound to the block and could resolve to a sibling's stale row.
 * For a template-backed launch — the block names the template, not a row, so
 * the stamp is the only thing binding the pane to its row — a failed create
 * aborts before anything is written. For a user agent the block already
 * names its row, and the create stays best-effort.
 */
export async function commitLaunch(opts: {
    isTemplate: boolean;
    meta: Record<string, unknown>;
    createInstance: () => Promise<{ id: string }>;
    setMeta: (meta: Record<string, unknown>) => Promise<unknown>;
    resync: () => Promise<unknown>;
    warn: (msg: string) => void;
}): Promise<{ ok: true; instanceId: string | null } | { ok: false; error: string }> {
    let instanceId: string | null = null;
    try {
        instanceId = (await opts.createInstance()).id;
    } catch (e: any) {
        const detail = e?.message ?? String(e);
        opts.warn(`agent instance row create failed: ${detail}`);
        if (opts.isTemplate) {
            return { ok: false, error: `Could not record this launch, so it was not started: ${detail}` };
        }
    }
    await opts.setMeta({ ...opts.meta, agentInstanceId: instanceId });
    await opts.resync();
    return { ok: true, instanceId };
}
