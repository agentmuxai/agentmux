// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, getApi, globalStore, WOS } from "@/app/store/global";
import { SignalAtom } from "@/util/util";
import { AgentViewWrapper } from "./agent-view";
import { PROVIDERS } from "./providers";
import { Logger } from "@/util/logger";

export class AgentViewModel implements ViewModel {
    viewType = "agent";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: SignalAtom<Block>;

    viewIcon: () => string;
    viewName: () => string;
    viewText: () => string | HeaderElem[];
    viewComponent: ViewComponent;
    noPadding: () => boolean;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
        this.viewComponent = AgentViewWrapper as any;

        this.viewIcon = () => "sparkles";
        this.viewName = () => "Agent";
        this.viewText = () => [] as HeaderElem[];
        this.noPadding = () => true;
    }

    /**
     * Launch an agent in presentation view.
     * For Phase 1, agentId maps to a provider ID (claude/codex/gemini).
     * Sets block metadata with CLI config and creates a SubprocessController.
     * The agent CLI is not started until the user sends the first message.
     */
    launchAgent = async (agentId: string): Promise<void> => {
        const provider = PROVIDERS[agentId];
        if (!provider) {
            Logger.error("agent", "Unknown agent", { agentId });
            return;
        }

        // Use bare command name (relies on PATH) — version-isolated installs are optional
        const cliBin = provider.cliCommand;

        Logger.info("agent", `Launching agent ${agentId} cmd=${cliBin}`, {
            agentId,
            styledArgs: provider.styledArgs,
            outputFormat: provider.styledOutputFormat,
        });

        const oref = WOS.makeORef("block", this.blockId);
        const blockId = this.blockId;

        // Build CLI args: -p for non-interactive, plus provider's streaming flags
        const cliArgs = ["-p", ...provider.styledArgs];

        // Build env vars: unset nested-session guards by setting them empty
        const envVars: Record<string, string> = {};
        if (provider.unsetEnv) {
            for (const key of provider.unsetEnv) {
                envVars[key] = "";
            }
        }

        try {
            // Store CLI config in block metadata for the backend to read on AgentInput
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: agentId,
                    agentOutputFormat: provider.styledOutputFormat,
                    controller: "subprocess",
                    cmd: cliBin,
                    "cmd:args": cliArgs,
                    "cmd:env": envVars,
                },
            });

            // Create SubprocessController (no-op start — waits for first message)
            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: globalStore.get(atoms.staticTabId),
                blockid: blockId,
                forcerestart: true,
            });
        } catch (e: any) {
            Logger.error("agent", "Failed to launch agent", { error: String(e) });
        }
    };

    /**
     * Launch a Forge-managed agent in presentation view.
     * Phase 1: Sets block metadata and loads content — enough to transition to presentation view.
     * Phase 2 (CLI resolution, auth, controller) happens in AgentPresentationView.onMount
     * so that log lines are visible to the user.
     */
    launchForgeAgent = async (agent: ForgeAgent): Promise<void> => {
        const provider = PROVIDERS[agent.provider];
        if (!provider) {
            Logger.error("agent", "Unknown provider in forge agent", { agentId: agent.id, provider: agent.provider });
            return;
        }

        Logger.info("agent", `Launching forge agent ${agent.name} (${agent.provider})`, {
            agentId: agent.id,
            provider: agent.provider,
        });

        // Load all content for this agent
        let contents: ForgeContent[] = [];
        try {
            contents = await RpcApi.GetAllForgeContentCommand(TabRpcClient, { agent_id: agent.id }) ?? [];
        } catch (e: any) {
            Logger.error("agent", "Failed to load forge content", { error: String(e) });
        }
        const contentMap: Record<string, string> = {};
        for (const c of contents) {
            contentMap[c.content_type] = c.content;
        }

        // Load skills for this agent (lazy-loading: only names/descriptions injected)
        let skills: ForgeSkill[] = [];
        try {
            skills = await RpcApi.ListForgeSkillsCommand(TabRpcClient, { agent_id: agent.id }) ?? [];
        } catch (e: any) {
            Logger.error("agent", "Failed to load forge skills", { error: String(e) });
        }

        // Determine working directory
        const workDir = agent.working_directory || `~/.agentmux/agents/${agent.name.toLowerCase().replace(/[^a-z0-9-_]/g, "-")}`;

        // Build CLI args: -p for non-interactive, plus provider's streaming flags, plus forge flags
        const cliArgs = ["-p", ...provider.styledArgs];
        if (agent.provider_flags) {
            cliArgs.push(...agent.provider_flags.split(/\s+/).filter(Boolean));
        }

        // Build env vars from provider unsetEnv + forge env content
        const envVars: Record<string, string> = {};
        if (provider.unsetEnv) {
            for (const key of provider.unsetEnv) {
                envVars[key] = "";
            }
        }
        if (contentMap["env"]) {
            for (const line of contentMap["env"].split("\n")) {
                const trimmed = line.trim();
                if (!trimmed || trimmed.startsWith("#")) continue;
                const eqIdx = trimmed.indexOf("=");
                if (eqIdx < 1) continue;
                const key = trimmed.substring(0, eqIdx);
                const val = trimmed.substring(eqIdx + 1);
                if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(key)) continue;
                envVars[key] = val;
            }
        }

        // Build config files to write via backend RPC
        const configFiles = buildConfigFiles(contentMap, skills);

        const oref = WOS.makeORef("block", this.blockId);
        try {
            // Store agent identity + CLI config in block metadata
            // cmd is set to bare command name initially; presentation view will resolve it
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: agent.id,
                    agentProvider: agent.provider,
                    agentOutputFormat: provider.styledOutputFormat,
                    agentName: agent.name,
                    agentIcon: agent.icon,
                    controller: "subprocess",
                    cmd: provider.cliCommand,
                    "cmd:args": cliArgs,
                    "cmd:cwd": workDir,
                    "cmd:env": envVars,
                },
            });

            // Write config files (CLAUDE.md, .mcp.json) to working directory via backend
            if (configFiles.length > 0) {
                await RpcApi.WriteAgentConfigCommand(TabRpcClient, {
                    working_dir: workDir,
                    files: configFiles,
                });
            }
        } catch (e: any) {
            Logger.error("agent", "Failed to launch forge agent", { error: String(e) });
        }
    };

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {}
}

/**
 * Resolve the version-isolated CLI install directory.
 */
function resolveCliDir(version: string, providerId: string): string {
    return `~/.agentmux/instances/v${version}/cli/${providerId}`;
}

/**
 * Build the list of config files to write to the agent working directory.
 * Assembles CLAUDE.md from soul + agentmd + memory + skills index,
 * and includes .mcp.json if present.
 */
function buildConfigFiles(
    contentMap: Record<string, string>,
    skills: ForgeSkill[] = []
): AgentConfigFile[] {
    const files: AgentConfigFile[] = [];

    // Build CLAUDE.md content: Soul + AgentMD + Memory + Skills Index
    const claudeMdParts: string[] = [];
    if (contentMap["soul"]) {
        claudeMdParts.push(contentMap["soul"]);
    }
    if (contentMap["agentmd"]) {
        if (claudeMdParts.length > 0) claudeMdParts.push("\n---\n");
        claudeMdParts.push(contentMap["agentmd"]);
    }
    if (contentMap["memory"]) {
        claudeMdParts.push("\n# Memory\n");
        claudeMdParts.push(contentMap["memory"]);
    }

    // Append skill index (lazy-loading: only names/descriptions, not full content)
    if (skills.length > 0) {
        claudeMdParts.push("\n# Available Skills\n\n");
        for (const skill of skills) {
            const triggerPart = skill.trigger ? ` (trigger: /${skill.trigger})` : "";
            const descPart = skill.description ? ` \u2014 ${skill.description}` : "";
            claudeMdParts.push(`- **${skill.name}**${triggerPart}${descPart}\n`);
        }
    }

    if (claudeMdParts.length > 0) {
        files.push({ path: "CLAUDE.md", content: claudeMdParts.join("") });
    }

    // Write .mcp.json if MCP content exists
    if (contentMap["mcp"]) {
        files.push({ path: ".mcp.json", content: contentMap["mcp"] });
    }

    return files;
}
