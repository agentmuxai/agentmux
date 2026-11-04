// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The steps of `AgentViewModel.launchAgentDefinition` (agent-model.ts) that stand on their own:
 * the agent's saved content, its working folder, the CLI's environment, its color, the block meta a
 * launch writes, the instance row and the account link. The method keeps the order of the steps and
 * every early return.
 */

import { RpcApi } from "@/app/store/rpc-api";
import type { AgentContent, AgentDefinition } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Logger } from "@/util/logger";
import type { AgentRuntimeConfig } from "../../agent-model/types";
import { ENV_CONTENT_TYPE, parseEnvText } from "./agent-env";
import { isValidAgentColor, pickAgentColor } from "./agent-color";
import { agentmuxHome, ensureProviderAuthDir } from "./agent-launch-env";
import { parseSeedZoom } from "./agent-zoom-seed";
import { CONTAINER_META_CLEARED_BY_A_HOST_LAUNCH } from "./leave-pane-agent";
import { launchController, PROVIDER_FLAGS_META_KEY, TERMINAL_PANE_META } from "./launch-args";
import { realAccountIdOrEmpty } from "./identity-carry-over";
import { shouldWriteThroughAccountLink } from "./identity-link-writethrough";
import { canonicalProviderId, lastLinkedAccountId } from "./providers/provider-id-aliases";
import { refreshAccountCache } from "@/app/view/identity/identity-model";
import { resolveVendorEnvOverride } from "./providers/vendor-env";
import { isContainerAgentMode } from "@/util/runtime-kind";
import type { LaunchOverrides } from "./launch-overrides";
import type { ProviderDefinition } from "./providers/types";

/** The agent's saved content (soul, agentmd, mcp, env, color, zoom) by content type. A failed load is logged and leaves it empty. */
export async function loadAgentContent(agentId: string): Promise<Record<string, string>> {
    let contents: AgentContent[] = [];
    try {
        contents = (await RpcApi.GetAllAgentContentCommand(TabRpcClient, { agent_id: agentId })) ?? [];
    } catch (e: any) {
        Logger.error("agent", "Failed to load agent content", { error: String(e) });
    }
    const contentMap: Record<string, string> = {};
    for (const c of contents) {
        contentMap[c.content_type] = c.content;
    }
    return contentMap;
}

/**
 * The folder the agent launches in. A seeded literal `~/.agentmux/...` path is rewritten to the
 * host-resolved `agentmuxHome()` (the backend rejects `~/` as a path traversal); a per-launch
 * instance gets its own folder under it, and an agent with no folder gets its slug's.
 */
export function launchWorkDir(agent: AgentDefinition, overrides: LaunchOverrides | undefined, slug: string, instanceSlug: string): string {
    const persisted = agent.working_directory ?? "";
    if (overrides?.instanceName) return `${agentmuxHome()}/agents/${instanceSlug}`;
    if (persisted.startsWith("~/.agentmux/")) return `${agentmuxHome()}${persisted.slice("~/.agentmux".length)}`;
    return persisted || `${agentmuxHome()}/agents/${slug}`;
}

/**
 * The environment of the agent's CLI: the provider's unset list, the agent's own env content, the
 * identity and git variables, the provider's auth dir and extras, and a model vendor override.
 * `isPersistent` is the EFFECTIVE mode: only a one-shot process (a container agent included) gets
 * an exit delay, since a persistent one must stay alive.
 */
export async function buildLaunchEnv(opts: {
    agent: AgentDefinition;
    provider: ProviderDefinition;
    slug: string;
    instanceSlug: string;
    instanceName: string;
    contentMap: Record<string, string>;
    isPersistent: boolean;
}): Promise<Record<string, string>> {
    const { agent, provider, slug, instanceSlug, instanceName, contentMap, isPersistent } = opts;
    const envVars: Record<string, string> = {};
    for (const key of provider.unsetEnv ?? []) {
        envVars[key] = "";
    }
    if (contentMap[ENV_CONTENT_TYPE]) {
        Object.assign(envVars, parseEnvText(contentMap[ENV_CONTENT_TYPE]).vars);
    }
    // Per-agent GitHub config isolation, keyed by the stable slug so renaming doesn't orphan it.
    envVars["GH_CONFIG_DIR"] = `${agentmuxHome()}/config/gh-${slug}`;
    // The stable role slug muxbus routes by (it must not change across respawns), the definition
    // slug, the working-directory suffix and a human-readable display name.
    envVars["AGENTMUX_AGENT_ID"] = slug;
    envVars["AGENTMUX_AGENT_SLUG"] = slug;
    envVars["AGENTMUX_INSTANCE_SLUG"] = instanceSlug;
    envVars["AGENTMUX_AGENT_DISPLAY"] = instanceName;
    // Git identity, so a host with no global git config doesn't fail with "Please tell me who you
    // are": the instance name for author and committer, the definition slug for the placeholder email.
    envVars["GIT_AUTHOR_NAME"] = instanceName;
    envVars["GIT_AUTHOR_EMAIL"] = `${slug}@agents.local`;
    envVars["GIT_COMMITTER_NAME"] = instanceName;
    envVars["GIT_COMMITTER_EMAIL"] = `${slug}@agents.local`;
    // Provider auth lives in the account-wide shared dir (~/.agentmux/shared/providers/<provider>/),
    // so one login serves every instance, channel and version. Skipped for a provider with no isolated auth dir.
    if (provider.authConfigDirEnvVar) {
        envVars[provider.authConfigDirEnvVar] = await ensureProviderAuthDir(provider.id);
    }
    if (provider.authExtraEnv) {
        Object.assign(envVars, provider.authExtraEnv);
    }
    // A model vendor override (ANTHROPIC_BASE_URL for a proxy, Bedrock, OpenRouter) only when the
    // agent has one AND the provider declares support for it; mirrors agent_open.rs.
    const vendorOverride = resolveVendorEnvOverride(agent.model_vendor_base_url, provider.baseUrlEnvVar);
    if (vendorOverride) {
        const [envVar, value] = vendorOverride;
        envVars[envVar] = value;
    }
    if (!isPersistent) {
        envVars["CLAUDE_CODE_EXIT_AFTER_STOP_DELAY"] = "30000";
    }
    return envVars;
}

/**
 * Allocate the working folder and write the config files, before `cmd:cwd` is set, so the meta
 * records the path the controller will really spawn in. Returns that path.
 *
 * A folder built here (a per-launch instance, or an agent with no folder) is eligible for `<base>-N`
 * collision suffixing; one the user set in `working_directory` is kept verbatim. A continuation
 * (`workDirOverride`) reuses the prior instance's folder and rewrites its config files, since
 * bundles can change between sessions.
 */
export async function allocateLaunchWorkDir(opts: {
    agent: AgentDefinition;
    overrides: LaunchOverrides | undefined;
    workDir: string;
    instanceName: string;
    agentMode: string;
}): Promise<string> {
    const { agent, overrides, workDir, instanceName, agentMode } = opts;
    const continueWorkDir = overrides?.workDirOverride?.trim() ?? "";
    const isContinue = continueWorkDir.length > 0;
    const autoAllocate = !isContinue && (Boolean(overrides?.instanceName) || (agent.working_directory ?? "").trim() === "");
    const writeWorkDir = isContinue ? continueWorkDir : workDir;
    const writeResult = await RpcApi.WriteAgentConfigCommand(TabRpcClient, {
        working_dir: writeWorkDir,
        // srv renders the launch files from the agent's definition, as `agent.open` does. The instance
        // name is what they call the agent, so the injected agentmux MCP server advertises the identity
        // the shell, git and pane title already use (#504).
        files: [],
        render_from_agent: true,
        instance_name: instanceName,
        auto_allocate: autoAllocate,
        // Picks which Operator Config entries the startup file gets.
        agent_type: agentMode,
        // The bundles picked for the agent follow Global Memory there.
        agent_id: agent.id,
    });
    return writeResult?.working_dir || writeWorkDir;
}

/**
 * The agent's border color: its saved `ui:color`, else one picked from its id and saved
 * (assign-if-missing, for an agent that predates the color migration).
 */
export function launchAgentColor(agent: AgentDefinition, contentMap: Record<string, string>): string {
    const rawColor = contentMap["ui:color"]?.trim();
    if (isValidAgentColor(rawColor)) return rawColor;
    const agentColor = pickAgentColor(agent.id);
    RpcApi.SetAgentContentCommand(TabRpcClient, {
        agent_id: agent.id,
        content_type: "ui:color",
        content: agentColor,
    }).catch((e: any) => {
        Logger.warn("agent", "Failed to persist assigned agent color", { error: String(e) });
    });
    return agentColor;
}

/**
 * The `AgentInstance` row of this launch: which pane runs which definition. `identity_id` and
 * `memory_id` are legacy column names for the caller's account and bundle picks; the instance name
 * and final folder let `listnamedagents` surface the instance later, and `parent_instance_id`
 * chains a continuation to the row it continues.
 */
export function createLaunchInstance(opts: {
    agent: AgentDefinition;
    blockId: string;
    overrides: LaunchOverrides | undefined;
    instanceName: string;
    finalWorkDir: string;
}) {
    const { agent, blockId, overrides, instanceName, finalWorkDir } = opts;
    return RpcApi.CreateAgentInstanceCommand(TabRpcClient, {
        definition_id: agent.id,
        block_id: blockId,
        identity_id: overrides?.accountId,
        memory_id: overrides?.bundleId,
        instance_name: instanceName,
        working_directory: finalWorkDir,
        parent_instance_id: overrides?.continueOfInstanceId,
    });
}

/**
 * The block meta a launch writes. `agent:sessionid` is set UNCONDITIONALLY (empty on a greenfield
 * launch): SetMeta merges, so an omitted key on a reused block would keep a prior block's session
 * and the backend would `--resume` the wrong conversation.
 */
export function buildLaunchMeta(opts: {
    agent: AgentDefinition;
    effectiveProvider: string;
    provider: ProviderDefinition;
    instanceName: string;
    agentMode: string;
    overrides: LaunchOverrides | undefined;
    paneFlags: string;
    cliBin: string | undefined;
    cliArgs: string[];
    finalWorkDir: string;
    envVars: Record<string, string>;
    runtimeConfig: AgentRuntimeConfig;
    continueSid: string;
    contentMap: Record<string, string>;
    agentColor: string;
}): Record<string, unknown> {
    const { agent, provider, overrides, agentMode, instanceName } = opts;
    return {
        agentId: agent.id,
        agentProvider: opts.effectiveProvider,
        agentOutputFormat: provider.styledOutputFormat,
        agentName: instanceName,
        agentIcon: agent.icon,
        // The mode `isPersistent` was derived from, so the controller/args choice and the mode
        // this block advertises can never disagree.
        agentMode,
        // Persisted so the per-turn cmd:args rebuild can reapply it; without it the user's flags are
        // dropped on the first send (#2872). `--fork-session` is a one-shot intent and is not persisted.
        [PROVIDER_FLAGS_META_KEY]: opts.paneFlags,
        // The image only when this agent has one. Otherwise the previous occupant's go (a host agent: all its
        // container keys): the merge would leave them, and a host agent in a once-container pane would read as one.
        ...(overrides?.containerImage || agent.container_image
            ? { "agent:container_image": overrides?.containerImage || agent.container_image }
            : isContainerAgentMode(agentMode)
              ? { "agent:container_image": null }
              : CONTAINER_META_CLEARED_BY_A_HOST_LAUNCH),
        // ACP harnesses (Copilot, OpenClaw, Pi) need srv's AcpController; a container agent keeps the
        // per-turn path into its container (SPEC_ACP_CLIENT_CONFORMANCE_2026_10_07.md).
        controller: launchController(provider, agentMode),
        // No host CLI path for a container agent: its CLI is in the container.
        ...(opts.cliBin ? { cmd: opts.cliBin } : {}),
        "cmd:args": opts.cliArgs,
        "cmd:cwd": opts.finalWorkDir,
        "cmd:env": opts.envVars,
        "agent:resume_flag": provider.resumeFlag ?? "",
        "agent:resume_strategy": provider.resumeStrategy ?? (provider.resumeFlag ? "flag" : "none"),
        "agent:session_id_field": provider.sessionIdField,
        "agent:prompt_arg_flag": provider.promptArgFlag ?? "",
        "agent:sessionid": opts.continueSid,
        "agent:runtime": opts.runtimeConfig,
        // Seeded from the agent's saved zoom, unconditionally (parseSeedZoom's null, not an omitted key).
        "term:zoom": parseSeedZoom(opts.contentMap["ui:zoom"]),
        "frame:activebordercolor": opts.agentColor,
        // A terminal-tier harness: this pane becomes a terminal running the CLI's own UI, in the agent's folder.
        ...(provider.controllerType === "terminal" ? { ...TERMINAL_PANE_META, "frame:title": instanceName } : {}),
    };
}

/**
 * Link the agent definition directly to the selected account for the launch's provider
 * (`agent_identity_link` upserts on `(agent_id, provider)`, so one call is correct). Best-effort:
 * the agent already started, so a failure is logged and the launch stands. The account id is checked
 * against a fresh account fetch (a legacy bundle id passes a shape-only check), so a stale
 * reference fails safe instead of tripping a foreign-key error.
 */
export async function linkLaunchAccount(agent: AgentDefinition, overrides: LaunchOverrides | undefined, effectiveProvider: string): Promise<void> {
    try {
        const accountId = realAccountIdOrEmpty(
            overrides?.accountId ?? "",
            (await refreshAccountCache()).map((a) => a.id),
        );
        const continuing = !!overrides?.continueOfInstanceId && !overrides?.forkSession;
        const links = !continuing
            ? null
            : await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: agent.id }).catch(() => null);
        // The link for THIS provider (a legacy-alias row counts), not any link the agent has.
        const linked = links == null ? null : lastLinkedAccountId(links, canonicalProviderId(effectiveProvider)) ?? "";
        if (shouldWriteThroughAccountLink({ accountId, continuing, linked })) {
            await RpcApi.LinkAgentIdentityCommand(TabRpcClient, {
                agent_id: agent.id,
                account_id: accountId,
                provider: effectiveProvider,
            });
        }
    } catch (e: any) {
        Logger.warn("agent", `direct identity link write-through failed: ${e?.message ?? String(e)}`);
    }
}
