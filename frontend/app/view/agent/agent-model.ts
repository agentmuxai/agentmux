// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { atoms, getApi, WOS } from "@/app/store/global";
import { SignalAtom } from "@/util/util";
import { AgentViewWrapper } from "./agent-view";
import { PROVIDERS, resolveProviderAlias } from "./providers";
import { Logger } from "@/util/logger";

export type OverlayTab = "forge" | "identity";

export class AgentViewModel implements ViewModel {
    viewType = "agent";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: SignalAtom<Block>;

    viewIcon: () => string;
    viewName: () => string;
    setViewName: (name: string) => Promise<void>;
    viewText: () => string | HeaderElem[];
    viewComponent: ViewComponent;
    noPadding: () => boolean;
    endIconButtons: () => IconButtonDecl[];
    nodejsError: string | null = null;

    // Callback wired by AgentPresentationView on mount so the title-bar
    // buttons can open the focused overlay without holding a SolidJS signal
    // in the model (signals must live inside the component tree).
    _setOverlayTab: ((tab: OverlayTab | null) => void) | null = null;
    // Last-used overlay tab — gear re-opens to whichever tab was active last.
    _lastOverlayTab: OverlayTab = "forge";

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
        this.viewComponent = AgentViewWrapper as any;

        // Drive the pane's title from block meta — launching an agent sets
        // `agentName` / `agentIcon` and the frame title automatically picks
        // them up via the blockAtom subscription. Before this, the title
        // was the literal string "Agent" regardless of which agent ran.
        // See SPEC_AGENT_PANE_FOLLOWUPS item #8.
        this.viewIcon = () => {
            const meta = this.blockAtom()?.meta;
            const icon = meta?.["agentIcon"];
            if (typeof icon === "string" && icon.length > 0) return icon;
            return "sparkles";
        };
        this.viewName = () => {
            const meta = this.blockAtom()?.meta;
            const name = meta?.["agentName"];
            if (typeof name === "string" && name.length > 0) return name;
            return "Agent";
        };
        this.viewText = () => [] as HeaderElem[];
        this.noPadding = () => true;
        this.setViewName = async (name: string) => {
            if (!name.trim()) return;
            const oref = WOS.makeORef("block", this.blockId);
            await RpcApi.SetMetaCommand(TabRpcClient, { oref, meta: { agentName: name.trim() } });
        };

        // Pane-frame header buttons: when an agent is loaded show ⚙ .
        // Hidden when no agent is loaded (picker screen).
        // Gear opens forge/identity panel; defaults to forge, remembers last tab.
        this.endIconButtons = () => {
            const agentId = this.blockAtom()?.meta?.["agentId"];
            if (!agentId) return [];
            return [
                {
                    elemtype: "iconbutton",
                    icon: "gear",
                    title: "Agent settings",
                    click: () => { this._setOverlayTab?.(this._lastOverlayTab); },
                },
            ];
        };
    }

    /**
     * Clear the agent-identity meta keys so AgentViewWrapper falls back
     * to the picker. Called from the pane-frame back button and from
     * useAgentCommands.back (which delegates here).
     */
    backToPicker = async (): Promise<void> => {
        const oref = WOS.makeORef("block", this.blockId);
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: null,
                    agentProvider: null,
                    agentOutputFormat: null,
                    agentName: null,
                    agentIcon: null,
                    agentCliPath: null,
                    agentCliArgs: null,
                    agentBinDir: null,
                    controller: null,
                },
            });
        } catch {
            // fail silently — user can manually switch via widget bar
        }
    };

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

        // Check Node.js availability for npm-based providers
        const nodejsError = await checkNodejsForProvider(provider.id);
        if (nodejsError) {
            this.nodejsError = nodejsError;
            Logger.error("agent", "Node.js not available", { agentId, error: nodejsError });
            return;
        }

        const version = getApi().getAboutModalDetails().version;
        const cliDir = resolveCliDir(version, provider.id);
        const cliBin = `${cliDir}/node_modules/.bin/${provider.cliCommand}`;

        Logger.info("agent", `Launching agent ${agentId} (v${version})`, {
            agentId,
            launchArgs: provider.launchArgs,
            outputFormat: provider.styledOutputFormat,
        });

        const oref = WOS.makeORef("block", this.blockId);
        const blockId = this.blockId;

        // Build CLI args: use persistent args if available, otherwise standard launch args
        const isPersistent = provider.controllerType === "persistent";
        const cliArgs = isPersistent && provider.persistentLaunchArgs
            ? [...provider.persistentLaunchArgs]
            : [...provider.launchArgs];

        // Build env vars: unset nested-session guards by setting them empty
        const envVars: Record<string, string> = {};
        if (provider.unsetEnv) {
            for (const key of provider.unsetEnv) {
                envVars[key] = "";
            }
        }

        // Provider auth isolation (skip if provider has no isolated auth dir configured)
        if (provider.authConfigDirEnvVar) {
            const authDir = await getApi().ensureAuthDir(provider.id);
            envVars[provider.authConfigDirEnvVar] = authDir;
        }
        if (provider.authExtraEnv) {
            Object.assign(envVars, provider.authExtraEnv);
        }
        // Only set exit delay for subprocess mode — persistent processes must stay alive
        if (provider.controllerType !== "persistent") {
            envVars["CLAUDE_CODE_EXIT_AFTER_STOP_DELAY"] = "30000";
        }

        try {
            // Store CLI config in block metadata for the backend to read on AgentInput
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: agentId,
                    agentOutputFormat: provider.styledOutputFormat,
                    controller: isPersistent ? "persistent" : "subprocess",
                    cmd: cliBin,
                    "cmd:args": cliArgs,
                    "cmd:env": envVars,
                    "agent:resume_flag": provider.resumeFlag ?? "",
                    "agent:session_id_field": provider.sessionIdField,
                },
            });

            // Create SubprocessController (no-op start — waits for first message)
            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: atoms.staticTabId(),
                blockid: blockId,
                forcerestart: true,
            });
        } catch (e: any) {
            Logger.error("agent", "Failed to launch agent", { error: String(e) });
        }
    };

    /**
     * Launch a Forge-managed agent in presentation view.
     * Uses the ForgeAgent's provider to look up CLI config.
     * Loads content blobs (soul, agentmd, mcp, env) and writes config files
     * to the working directory via WriteAgentConfigCommand, then creates
     * a SubprocessController ready for user input.
     */
    launchForgeAgent = async (agent: ForgeAgent): Promise<void> => {
        const provider = PROVIDERS[agent.provider] ?? PROVIDERS[resolveProviderAlias(agent.provider)];
        if (!provider) {
            Logger.error("agent", "Unknown provider in forge agent", { agentId: agent.id, provider: agent.provider });
            return;
        }

        // Check Node.js availability for npm-based providers
        const nodejsError = await checkNodejsForProvider(provider.id);
        if (nodejsError) {
            this.nodejsError = nodejsError;
            Logger.error("agent", "Node.js not available for forge agent", { agentId: agent.id, error: nodejsError });
            return;
        }

        const version = getApi().getAboutModalDetails().version;
        const cliDir = resolveCliDir(version, provider.id);
        const cliBin = `${cliDir}/node_modules/.bin/${provider.cliCommand}`;

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

        // Determine working directory. Use the agent's stable slug
        // (Step 1 of SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md) so
        // renaming the agent doesn't move the directory on disk.
        // Falls back to the legacy name-derived form if slug is empty
        // (defensive — the v4 migration backfills slug for every row).
        const slug = agent.slug || agent.name.toLowerCase().replace(/[^a-z0-9-_]/g, "-");
        const workDir = agent.working_directory || `${agentmuxHome()}/agents/${slug}`;

        // Build CLI args: use persistent args if available, otherwise standard launch args
        const isPersistent = provider.controllerType === "persistent";
        const cliArgs = isPersistent && provider.persistentLaunchArgs
            ? [...provider.persistentLaunchArgs]
            : [...provider.launchArgs];
        if (agent.provider_flags) {
            cliArgs.push(...agent.provider_flags.split(/\s+/).filter(Boolean));
        }

        // Build env vars from provider unsetEnv + forge env content + per-agent isolation
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

        // Per-agent GitHub config isolation — keyed by the stable
        // slug so renaming doesn't orphan ~/.agentmux/config/gh-<old>.
        envVars["GH_CONFIG_DIR"] = `${agentmuxHome()}/config/gh-${slug}`;

        // AGENTMUX_AGENT_ID stays as the display name for backwards
        // compat: shell integration scripts (bash.sh / zsh.sh / pwsh.ps1)
        // emit OSC sequences carrying this value as the terminal pane
        // label, and agentmux-mcp routes inter-agent messages by it.
        // Flipping to slug here would change visible labels and break
        // routing — that's a separate coordinated migration.
        envVars["AGENTMUX_AGENT_ID"] = agent.name;
        // New, additive: the stable slug (Step 2 of identity
        // restructure). Downstream code can opt into reading this
        // when it needs the rename-stable form.
        envVars["AGENTMUX_AGENT_SLUG"] = slug;
        // Explicit display alias. Today identical to AGENTMUX_AGENT_ID
        // — once Step 4's downstream coordination flips ID to the
        // slug, DISPLAY remains the human-readable label.
        envVars["AGENTMUX_AGENT_DISPLAY"] = agent.name;

        // Git identity — prevents "Please tell me who you are" errors when
        // the host machine has no global git config. Derived from the agent's
        // display name + slug-based placeholder email. The Identity panel can
        // supply a real email in a follow-on, but this fallback is safe and
        // satisfies git's format requirement unconditionally.
        envVars["GIT_AUTHOR_NAME"]     = agent.name;
        envVars["GIT_AUTHOR_EMAIL"]    = `${slug}@agents.local`;
        envVars["GIT_COMMITTER_NAME"]  = agent.name;
        envVars["GIT_COMMITTER_EMAIL"] = `${slug}@agents.local`;
        // GIT_CONFIG_GLOBAL is intentionally not set: we use the 4 identity
        // env vars above which git always honours, avoiding any path-handling edge cases.

        // Provider auth isolation: shared per-version auth dir (not per-agent)
        // Each AgentMux version gets its own auth space via the Tauri app data dir,
        // which already includes the version in its identifier (ai.agentmux.app.vX-Y-Z).
        // Skip if provider has no isolated auth dir configured (e.g. Claude uses ~/.claude/ globally).
        if (provider.authConfigDirEnvVar) {
            const authDir = await getApi().ensureAuthDir(provider.id);
            envVars[provider.authConfigDirEnvVar] = authDir;
        }
        if (provider.authExtraEnv) {
            Object.assign(envVars, provider.authExtraEnv);
        }
        // Only set exit delay for subprocess mode — persistent processes must stay alive
        if (provider.controllerType !== "persistent") {
            envVars["CLAUDE_CODE_EXIT_AFTER_STOP_DELAY"] = "30000";
        }

        // Build config files to write via backend RPC
        const configFiles = buildConfigFiles(contentMap, skills, agent);

        const oref = WOS.makeORef("block", this.blockId);
        const blockId = this.blockId;
        try {
            // Store CLI config in block metadata
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: agent.id,
                    agentProvider: agent.provider,
                    agentOutputFormat: provider.styledOutputFormat,
                    agentName: agent.name,
                    agentIcon: agent.icon,
                    agentMode: agent.agent_type || "host",
                    controller: isPersistent ? "persistent" : "subprocess",
                    cmd: cliBin,
                    "cmd:args": cliArgs,
                    "cmd:cwd": workDir,
                    "cmd:env": envVars,
                    "agent:resume_flag": provider.resumeFlag ?? "",
                    "agent:session_id_field": provider.sessionIdField,
                },
            });

            // Write config files (CLAUDE.md, .mcp.json) to working directory via backend
            if (configFiles.length > 0) {
                await RpcApi.WriteAgentConfigCommand(TabRpcClient, {
                    working_dir: workDir,
                    files: configFiles,
                });
            }

            // Create SubprocessController (no-op start — waits for first message)
            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: atoms.staticTabId(),
                blockid: blockId,
                forcerestart: true,
            });
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
 * Check if Node.js is available. Required for npm-based providers (Codex, Gemini).
 * Claude has its own standalone installer and doesn't need Node.js.
 * Returns null if Node.js is available or not needed, or an error message string.
 */
async function checkNodejsForProvider(providerId: string): Promise<string | null> {
    if (providerId === "claude") return null; // Claude has standalone installer
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
function agentmuxHome(): string {
    const fromHost = getApi().getUserHomeDir();
    if (fromHost) return fromHost;
    const home = getApi().getEnv("HOME") || getApi().getEnv("USERPROFILE") || "~";
    return `${home}/.agentmux`;
}

/**
 * Resolve the version-isolated CLI install directory.
 */
function resolveCliDir(version: string, providerId: string): string {
    return `${agentmuxHome()}/instances/v${version}/cli/${providerId}`;
}

/**
 * Build the list of config files to write to the agent working directory.
 * Assembles CLAUDE.md from soul + agentmd + memory + skills index,
 * writes each skill as a slash command in .claude/commands/,
 * writes hooks.json if present, auto-injects AgentMux MCP server,
 * and applies template variable substitution.
 */
function buildConfigFiles(
    contentMap: Record<string, string>,
    skills: ForgeSkill[] = [],
    agent?: ForgeAgent
): AgentConfigFile[] {
    const files: AgentConfigFile[] = [];

    // Template variables for {{}} substitution
    const templateVars: Record<string, string> = {};
    if (agent) {
        templateVars["AGENT"] = agent.name;
        templateVars["AGENT_DISPLAY"] = agent.name;
        templateVars["AGENT_SLUG"] = agent.slug || agent.name.toLowerCase().replace(/[^a-z0-9-_]/g, "-");
        templateVars["WORKING_DIR"] = agent.working_directory || "";
        templateVars["AGENT_ID"] = agent.id;
    }
    templateVars["DATE"] = new Date().toISOString().slice(0, 10);

    // Build CLAUDE.md content: Soul + AgentMD + Memory + Skills Index
    const claudeMdParts: string[] = [];
    if (contentMap["soul"]) {
        claudeMdParts.push(expandTemplate(contentMap["soul"], templateVars));
    }
    if (contentMap["agentmd"]) {
        if (claudeMdParts.length > 0) claudeMdParts.push("\n---\n");
        claudeMdParts.push(expandTemplate(contentMap["agentmd"], templateVars));
    }
    if (contentMap["memory"]) {
        claudeMdParts.push("\n# Memory\n");
        claudeMdParts.push(contentMap["memory"]);
    }

    // Append skill index with trigger references
    if (skills.length > 0) {
        claudeMdParts.push("\n# Available Skills\n\n");
        claudeMdParts.push("Use `/<trigger>` to invoke a skill.\n\n");
        for (const skill of skills) {
            const triggerPart = skill.trigger ? ` (trigger: /${skill.trigger})` : "";
            const descPart = skill.description ? ` \u2014 ${skill.description}` : "";
            claudeMdParts.push(`- **${skill.name}**${triggerPart}${descPart}\n`);
        }
    }

    if (claudeMdParts.length > 0) {
        files.push({ path: "CLAUDE.md", content: claudeMdParts.join("") });
    }

    // Write each skill as a slash command: .claude/commands/{trigger}.md
    for (const skill of skills) {
        if (skill.trigger && skill.content) {
            const content = expandTemplate(skill.content, templateVars);
            files.push({ path: `.claude/commands/${skill.trigger}.md`, content });
        }
    }

    // Write hooks.json if hooks content exists
    if (contentMap["hooks"]) {
        files.push({ path: ".claude/hooks.json", content: contentMap["hooks"] });
    }

    // Build .mcp.json: auto-inject AgentMux MCP + merge user-provided config
    const mcpConfig = buildMcpConfig(contentMap["mcp"], agent);
    if (mcpConfig) {
        files.push({ path: ".mcp.json", content: mcpConfig });
    }

    return files;
}

/**
 * Replace {{VARIABLE}} placeholders in content with values from vars map.
 */
function expandTemplate(content: string, vars: Record<string, string>): string {
    return content.replace(/\{\{(\w+)\}\}/g, (match, key) => {
        return vars[key] ?? match;
    });
}

/**
 * Build .mcp.json content with auto-injected AgentMux MCP server.
 * Merges with user-provided MCP config if present.
 */
function buildMcpConfig(userMcpContent: string | undefined, agent?: ForgeAgent): string | null {
    // Auto-inject AgentMux MCP server for inter-agent messaging
    const agentMuxServer: Record<string, any> = {
        type: "stdio",
        command: "agentmux-mcp",
        args: [],
        env: {} as Record<string, string>,
    };
    if (agent) {
        agentMuxServer.env["AGENTMUX_AGENT_ID"] = agent.name;
        if (agent.agent_bus_id) {
            agentMuxServer.env["AGENTMUX_AGENT_BUS_ID"] = agent.agent_bus_id;
        }
    }

    let mcpObj: Record<string, any> = { mcpServers: { agentmux: agentMuxServer } };

    // Merge user-provided MCP config
    if (userMcpContent) {
        try {
            const userMcp = JSON.parse(userMcpContent);
            if (userMcp.mcpServers) {
                mcpObj.mcpServers = { ...mcpObj.mcpServers, ...userMcp.mcpServers };
            }
        } catch {
            // If user MCP isn't valid JSON, skip merge but still write auto-injected
            Logger.error("agent", "Invalid MCP JSON in forge content, using auto-injected only");
        }
    }

    return JSON.stringify(mcpObj, null, 2);
}
