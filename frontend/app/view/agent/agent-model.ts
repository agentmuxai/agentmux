// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, getApi, globalStore, WOS } from "@/app/store/global";
import { atom, Atom, PrimitiveAtom } from "jotai";
import React from "react";
import { AgentViewWrapper } from "./agent-view";
import { PROVIDERS } from "./providers";
import { buildBootstrapScript, guessShellType } from "./bootstrap";
import { Logger } from "@/util/logger";
import { stringToBase64 } from "@/util/util";

export class AgentViewModel implements ViewModel {
    viewType = "agent";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: Atom<Block>;

    viewIcon: Atom<string>;
    viewName: Atom<string>;
    viewText: Atom<string | HeaderElem[]>;
    viewComponent: ViewComponent;
    noPadding = atom(true);

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
        this.viewComponent = AgentViewWrapper as any;

        this.viewIcon = atom("sparkles");
        this.viewName = atom("Agent");
        this.viewText = atom<string | HeaderElem[]>([]);
    }

    /**
     * Launch an agent in presentation view.
     * For Phase 1, agentId maps to a provider ID (claude/codex/gemini).
     * In Phase 2 this will be a Forge agent UUID.
     */
    launchAgent = async (agentId: string): Promise<void> => {
        const provider = PROVIDERS[agentId];
        if (!provider) {
            Logger.error("agent", "Unknown agent", { agentId });
            return;
        }

        const version = getApi().getAboutModalDetails().version;
        const shellType = guessShellType(getApi().getPlatform());

        Logger.info("agent", `Launching agent ${agentId} (v${version})`, {
            agentId,
            shellType,
            styledArgs: provider.styledArgs,
            outputFormat: provider.styledOutputFormat,
        });

        const oref = WOS.makeORef("block", this.blockId);
        const blockId = this.blockId;
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    "agentId": agentId,
                    "agentOutputFormat": provider.styledOutputFormat,
                    "controller": "shell",
                },
            });

            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: globalStore.get(atoms.staticTabId),
                blockid: blockId,
                forcerestart: true,
            });

            setTimeout(async () => {
                const script = buildBootstrapScript({
                    version,
                    provider,
                    shellType,
                    args: provider.styledArgs,
                });
                const b64data = stringToBase64(script + "\r");
                await RpcApi.ControllerInputCommand(TabRpcClient, {
                    blockid: blockId,
                    inputdata64: b64data,
                });
            }, 500);
        } catch (e: any) {
            Logger.error("agent", "Failed to launch agent", { error: String(e) });
        }
    };

    /**
     * Launch a Forge-managed agent in presentation view.
     * Uses the ForgeAgent's provider to look up CLI config.
     * Loads content blobs (soul, agentmd, mcp, env) and writes config files
     * to the working directory before bootstrapping the agent CLI.
     */
    launchForgeAgent = async (agent: ForgeAgent): Promise<void> => {
        const provider = PROVIDERS[agent.provider];
        if (!provider) {
            Logger.error("agent", "Unknown provider in forge agent", { agentId: agent.id, provider: agent.provider });
            return;
        }

        const version = getApi().getAboutModalDetails().version;
        const shellType = guessShellType(getApi().getPlatform());

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

        const oref = WOS.makeORef("block", this.blockId);
        const blockId = this.blockId;
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: agent.id,
                    agentProvider: agent.provider,
                    agentOutputFormat: provider.styledOutputFormat,
                    agentName: agent.name,
                    agentIcon: agent.icon,
                    controller: "shell",
                },
            });

            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: globalStore.get(atoms.staticTabId),
                blockid: blockId,
                forcerestart: true,
            });

            setTimeout(async () => {
                // Build config-writing preamble
                const preamble = buildConfigPreamble(contentMap, workDir, agent, skills);

                const script = buildBootstrapScript({
                    version,
                    provider,
                    shellType,
                    args: provider.styledArgs,
                    preamble,
                    cwd: workDir,
                    extraFlags: agent.provider_flags || undefined,
                });
                const b64data = stringToBase64(script + "\r");
                await RpcApi.ControllerInputCommand(TabRpcClient, {
                    blockid: blockId,
                    inputdata64: b64data,
                });
            }, 500);
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
 * Build shell commands to write config files before agent bootstrap.
 * Creates the working directory, writes CLAUDE.md (soul + agentmd + memory),
 * .mcp.json, and sets environment variables.
 */
function buildConfigPreamble(
    contentMap: Record<string, string>,
    workDir: string,
    agent: ForgeAgent,
    skills: ForgeSkill[] = []
): string {
    const lines: string[] = [];

    // Create working directory
    lines.push(`mkdir -p ${shellEscape(workDir)}`);

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
        const claudeMd = claudeMdParts.join("");
        writeHeredoc(lines, `${shellEscape(workDir)}/CLAUDE.md`, claudeMd);
    }

    // Write .mcp.json if MCP content exists
    if (contentMap["mcp"]) {
        writeHeredoc(lines, `${shellEscape(workDir)}/.mcp.json`, contentMap["mcp"]);
    }

    // Set environment variables from env content (key=value format, one per line)
    if (contentMap["env"]) {
        const envLines = contentMap["env"].split("\n").filter((l) => l.includes("=") && !l.startsWith("#"));
        for (const envLine of envLines) {
            const trimmed = envLine.trim();
            if (!trimmed) continue;
            // Validate KEY=VALUE format: key must be alphanumeric/underscore,
            // value is single-quoted to prevent injection
            const eqIdx = trimmed.indexOf("=");
            if (eqIdx < 1) continue;
            const key = trimmed.substring(0, eqIdx);
            const val = trimmed.substring(eqIdx + 1);
            if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(key)) continue;
            lines.push(`export ${key}=${shellQuote(val)}`);
        }
    }

    // cd to working directory
    lines.push(`cd ${shellEscape(workDir)}`);

    return lines.join("\n");
}

/**
 * Write content to a file using a heredoc with a unique delimiter.
 * Generates a delimiter that doesn't appear in the content to prevent
 * heredoc injection attacks.
 */
function writeHeredoc(lines: string[], path: string, content: string): void {
    let delimiter = "FORGE_EOF";
    let suffix = 0;
    // Ensure delimiter doesn't appear as a standalone line in content
    while (content.split("\n").some((line) => line.trim() === delimiter)) {
        suffix++;
        delimiter = `FORGE_EOF_${suffix}`;
    }
    lines.push(`cat > ${path} << '${delimiter}'`);
    lines.push(content);
    lines.push(delimiter);
}

/** Single-quote a value for safe shell use (escapes embedded single quotes) */
function shellQuote(s: string): string {
    return "'" + s.replace(/'/g, "'\\''") + "'";
}

/** Shell escaping for paths — uses single quotes for safety */
function shellEscape(s: string): string {
    if (/^[a-zA-Z0-9_.\/-]+$/.test(s)) return s;
    return shellQuote(s);
}
