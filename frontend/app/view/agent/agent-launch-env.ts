// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Launch-environment helpers extracted from agent-model.ts (see
 * docs/specs — modularization pass, 2026-07-23). These resolve
 * host-level paths/availability needed before spawning an agent CLI.
 * No `this`/class coupling — standalone functions the model calls into.
 */

import { srvInfo } from "@/app/store/srv-info";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Logger } from "@/util/logger";
import { DEFAULT_RUNTIME_CONFIG, type AgentRuntimeConfig } from "./types";
import type { ProviderDefinition, ProviderModel } from "./providers/types";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { markAgentOpen } from "./open-trace";
import { parseProviderFlags, withoutOverriddenFlags } from "./launch-args";
import { effortFromFlags, modelFromFlags, modelTakesEffort, permissionModeFromFlags } from "./runtime-capabilities";

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
 * A provider's default auth/config dir (`~/.agentmux/shared/providers/<id>/`),
 * created and isolation-prepared by srv (`provider.ensureauthdir`) — the same
 * step srv's own agent open runs, e.g. Claude's CLAUDE.md placeholder. The
 * value for the provider's `authConfigDirEnvVar`.
 */
export async function ensureProviderAuthDir(providerId: string): Promise<string> {
    const { path } = await RpcApi.EnsureProviderAuthDirCommand(TabRpcClient, { provider_id: providerId });
    return path;
}

/**
 * Return the AgentMux user-home base directory, as an absolute path on srv's
 * machine: the paths built from it (an agent's working dir, `GH_CONFIG_DIR`,
 * the widgets dir) are srv's. srv reports it on connect (`srvinfo`,
 * docs/specs/SPEC_SRV_INFO_ON_CONNECT_2026_10_09.md), so it lands in the right
 * place for the instance type:
 *   - Portable: `<portable>/data`
 *   - Installed: `~/.agentmux`
 *   - `AGENTMUX_DATA_HOME` env override: wins over both.
 *
 * srv sends `srvinfo` on the WebSocket before it answers any RPC, the UI
 * records it as it arrives (`noteSrvInfoMessage`), and startup waits for a
 * WebSocket RPC reply (`GetFullConfigCommand`, app-init.ts `initMux`) before
 * anything here runs, so the value is there. Throws rather than guess a path
 * if it isn't.
 *
 * See `docs/specs/portable-agent-working-dirs.md`.
 */
export function agentmuxHome(): string {
    const home = srvInfo()?.homeDir;
    if (!home) throw new Error("agentmuxHome: srv hasn't reported its AgentMux home directory yet");
    return home;
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
export async function resolveCliBin(
    provider: ProviderDefinition,
    blockId: string,
    // The pane whose open this is part of, for its `[agent-open]` line
    // (open-trace.ts) — the launch target, which a quick fork makes a
    // different block from `blockId` (ReAgent P1 on #3939).
    traceBlockId: string = blockId,
    // The agent being launched, for its last-run CLI version record: a new
    // pane resolves before its `agentId` meta is written (cli_notice.rs).
    agentId?: string,
): Promise<string> {
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
            // CLI notices go to the pane being launched, not a quick
            // fork's source pane.
            notice_block_id: traceBlockId,
            agent_id: agentId,
        },
        // Same budget as launch-flow.ts: a first launch may npm-install.
        { timeout: 300000 },
    );
    if (!result?.cli_path) {
        throw new Error(`ResolveCli returned no CLI path for provider '${provider.id}'`);
    }
    markAgentOpen(traceBlockId, "cli", { cli_source: result.source });
    return result.cli_path;
}

/**
 * The provider a launch runs: the agent's own. A bundle carries no harness
 * (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §3.1); the agent owns it,
 * read-only after creation. Mirrors the server's
 * `resolve_effective_provider_id`, which spawn and the credential gate use,
 * so the CLI launched here and the credentials checked there agree.
 *
 * The bound bundle's `provider` is consulted only when the agent has none,
 * a definition the server's migration couldn't reach; there it is the value
 * the agent has always run with. Any failure falls back to `agent.provider`;
 * this never blocks a launch on its own.
 */
export async function resolveEffectiveLaunchProvider(agent: AgentDefinition): Promise<string> {
    if (agent.provider || !agent.memory_id) return agent.provider;
    try {
        const bundle = await RpcApi.GetBundleCommand(TabRpcClient, { id: agent.memory_id });
        return bundle?.provider ?? "";
    } catch (e: any) {
        Logger.warn("agent", "Failed to read the bound bundle for an agent with no provider", {
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
    /**
     * The agent definition's own `provider_flags`. Its `--model` / `--effort` are
     * what the agent is DEFINED to run, so they seed the menu: otherwise the menu
     * would start on the catalog default while the process runs the definition's.
     * Its `--permission-mode` (and `--dangerously-skip-permissions` / `--yolo`,
     * which read as "bypass") seeds the mode the same way.
     * An explicit model chosen at launch still wins.
     */
    providerFlags?: unknown,
    /**
     * A runtime to carry over from another pane (a fork of a running agent): the
     * fork should start as the source was running, not on the defaults. Applies
     * to each setting it names; a model chosen at launch still wins over it.
     */
    carryOver?: Partial<AgentRuntimeConfig>,
): AgentRuntimeConfig {
    const flags = parseProviderFlags(providerFlags);
    const model =
        overridesModel ||
        carryOver?.model ||
        modelFromFlags(flags) ||
        providerModels?.find((m) => m.default)?.value ||
        DEFAULT_RUNTIME_CONFIG.model;
    const effort = carryOver?.effort ?? effortFromFlags(flags) ?? DEFAULT_RUNTIME_CONFIG.effort;
    const permissionMode =
        carryOver?.permissionMode ?? permissionModeFromFlags(flags) ?? DEFAULT_RUNTIME_CONFIG.permissionMode;
    return {
        ...DEFAULT_RUNTIME_CONFIG,
        model,
        effort: effort as AgentRuntimeConfig["effort"],
        permissionMode: permissionMode as AgentRuntimeConfig["permissionMode"],
    };
}

/**
 * The runtime a launch starts with, and the provider flags its pane keeps.
 *
 * Precedence, setting by setting: a choice made in the launch modal
 * (`overridesModel`), then the runtime of the pane being forked
 * (`forkCarryOver`), then what the user last picked for this agent
 * (`remembered`), then the definition's own flags, then the provider default.
 *
 * The definition's flags are appended after the runtime's, so one that names a
 * setting decided above would win over it: the menu would show the choice and
 * the process run the definition's. Those flags are taken out of the PANE's copy
 * (`paneFlags`; the definition itself is untouched). With nothing to take out,
 * the flags are returned exactly as the definition has them.
 */
export function resolveLaunchRuntime(
    overridesModel: string | undefined,
    provider: Pick<ProviderDefinition, "id" | "models">,
    providerFlags: unknown,
    remembered: Partial<AgentRuntimeConfig> | undefined,
    forkCarryOver: Partial<AgentRuntimeConfig> | undefined,
): { runtimeConfig: AgentRuntimeConfig; paneFlags: string } {
    const carryOver = { ...remembered, ...forkCarryOver };
    const runtimeConfig = resolveInitialRuntimeConfig(overridesModel, provider.models, providerFlags, carryOver);
    const stripped = withoutOverriddenFlags(providerFlags, {
        model: !!(overridesModel || carryOver.model),
        // A model that takes no --effort (Haiku) must not keep the definition's
        // either: it is appended last and the CLI is handed a flag it does not take.
        effort: carryOver.effort !== undefined || (provider.id === "claude" && !modelTakesEffort(runtimeConfig.model)),
        permissionMode: carryOver.permissionMode !== undefined,
    });
    const paneFlags =
        stripped === parseProviderFlags(providerFlags).join(" ") ? ((providerFlags as string | undefined) ?? "") : stripped;
    return { runtimeConfig, paneFlags };
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
