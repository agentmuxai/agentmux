// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { atoms, getApi, WOS } from "@/app/store/global";
import { SignalAtom } from "@/util/util";
import { AgentViewWrapper } from "./agent-view";
import { buildAgentPaneIcon } from "./components/AgentPaneIcon";
import { PROVIDERS, resolveProviderAlias } from "./providers";
import { Logger } from "@/util/logger";
import { buildInstanceSlug } from "./defaults/instance-slug";
import type { LaunchOverrides } from "./components/AgentLaunchModal";

export type OverlayTab = "forge" | "identity";

export class AgentViewModel implements ViewModel {
    viewType = "agent";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: SignalAtom<Block>;

    viewIcon: () => string | IconButtonDecl;
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
        this.viewIcon = (): string | IconButtonDecl => {
            const meta = this.blockAtom()?.meta;
            const provider = meta?.["agentProvider"];
            if (typeof provider === "string" && provider.length > 0) {
                // Forge agents may store an alias (e.g. "kimi-cli") rather
                // than the canonical provider key ProviderLogo matches on.
                return buildAgentPaneIcon(resolveProviderAlias(provider));
            }
            // Quick-launch path (`launchAgent`) writes `agentId` as the
            // provider key — show the brand logo for that path too.
            const agentId = meta?.["agentId"];
            if (typeof agentId === "string" && PROVIDERS[agentId]) {
                return buildAgentPaneIcon(agentId);
            }
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
     * For Phase 1, agentId maps to a provider ID (claude/codex/gemini/kimi/openclaw/pi).
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
    launchForgeAgent = async (agent: ForgeAgent, overrides?: LaunchOverrides): Promise<void> => {
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

        // Definition slug: stable across launches of this definition.
        // Used for identity-scoped paths (GH_CONFIG_DIR, git author
        // email) so credentials and identity persist even when the
        // user relaunches the same definition with different instance
        // names. See SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md §1.
        const slug = agent.slug || agent.name.toLowerCase().replace(/[^a-z0-9-_]/g, "-");

        // Instance name: overrides.instanceName wins (modal-supplied);
        // falls back to the definition's own name for callers that
        // haven't adopted the modal flow yet (legacy / tests).
        const instanceName = overrides?.instanceName?.trim() || agent.name;

        // Per-launch slug: `<slug>-<YYYYMMDD-HHMMSS>`. Ensures two
        // instances of the same definition never share a working
        // directory even if they're launched with the same name.
        // Definitions without an override retain the legacy
        // slug-only path to avoid churn on existing seeded agents.
        const instanceSlug = overrides?.instanceName
            ? buildInstanceSlug(overrides.instanceName)
            : slug;

        // If the persisted working_directory was seeded with a literal `~/.agentmux/...`
        // path (see `scripts/gen-seed.js`), rewrite its prefix to the host-resolved
        // `agentmuxHome()`. On portable builds that points into the portable data dir;
        // on installed builds it matches the original `~/.agentmux/` verbatim. Without
        // this rewrite the backend rejects the path with "path traversal denied"
        // because `~/` never gets expanded to an absolute path at the OS layer.
        const persisted = agent.working_directory ?? "";
        const workDir = overrides?.instanceName
            ? `${agentmuxHome()}/agents/${instanceSlug}`
            : persisted.startsWith("~/.agentmux/")
                ? `${agentmuxHome()}${persisted.slice("~/.agentmux".length)}`
                : (persisted || `${agentmuxHome()}/agents/${slug}`);

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

        // AGENTMUX_AGENT_ID: the instance name (modal-supplied or
        // definition-default). Shell integration scripts emit this
        // as the terminal pane label, and agentmux-mcp routes
        // inter-agent messages by it.
        envVars["AGENTMUX_AGENT_ID"] = instanceName;
        // Stable definition slug — survives renames and re-launches.
        // Downstream code reads this when it needs the rename-stable
        // form (e.g. session-id lookup).
        envVars["AGENTMUX_AGENT_SLUG"] = slug;
        // Per-instance slug: the working-directory suffix. Lets
        // downstream code find the instance's scratch dir without
        // re-parsing paths.
        envVars["AGENTMUX_INSTANCE_SLUG"] = instanceSlug;
        // Explicit display alias. Human-readable, unaffected by
        // slug vs id collapsing in future migrations.
        envVars["AGENTMUX_AGENT_DISPLAY"] = instanceName;

        // Git identity — prevents "Please tell me who you are" errors when
        // the host machine has no global git config. Uses the instance
        // name for author/committer and the stable definition slug for
        // the placeholder email so commits attribute to "this run" while
        // still grouping by definition identity.
        envVars["GIT_AUTHOR_NAME"]     = instanceName;
        envVars["GIT_AUTHOR_EMAIL"]    = `${slug}@agents.local`;
        envVars["GIT_COMMITTER_NAME"]  = instanceName;
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

        // Build config files to write via backend RPC. Pass the
        // resolved instance name so the injected agentmux MCP server
        // advertises the same identity the shell / git / pane title
        // already use — otherwise inter-agent messaging routes on
        // the definition name while everything else advertises the
        // instance name (caught by codex on PR #504).
        const configFiles = buildConfigFiles(contentMap, skills, agent, instanceName);

        const oref = WOS.makeORef("block", this.blockId);
        const blockId = this.blockId;
        try {
            // Whether the work_dir was constructed by us (and is thus
            // eligible for `<base>-N` collision suffixing) or was
            // pulled verbatim from the agent definition's
            // `working_directory` field (which we must NEVER rewrite,
            // even if it happens to live under ~/.agentmux/ — a user
            // can legitimately set `agent.working_directory` to
            // `~/.agentmux/my-project` and expect that exact dir).
            //
            // Auto cases:
            //   - overrides.instanceName given (modal launch chose a
            //     fresh per-launch slug)
            //   - working_directory is empty (we filled in the legacy
            //     ~/.agentmux/agents/<slug> default ourselves)
            // Any other persisted value is user-specified and stays
            // verbatim through allocation.
            const autoAllocate =
                Boolean(overrides?.instanceName) ||
                (agent.working_directory ?? "").trim() === "";

            // Allocate the workdir + write config files BEFORE we set
            // cmd:cwd, so the meta records the actual collision-
            // resolved path the controller will spawn the CLI in.
            // Always call WriteAgentConfigCommand (even with no files
            // to write) when auto-allocate so we get the atomic
            // `mkdir` reservation. Returns the final path.
            const writeResult = await RpcApi.WriteAgentConfigCommand(TabRpcClient, {
                working_dir: workDir,
                files: configFiles,
                auto_allocate: autoAllocate,
            });
            const finalWorkDir = writeResult?.working_dir || workDir;

            // Store CLI config in block metadata using the (possibly
            // collision-resolved) finalWorkDir.
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: agent.id,
                    agentProvider: agent.provider,
                    agentOutputFormat: provider.styledOutputFormat,
                    agentName: instanceName,
                    agentIcon: agent.icon,
                    agentMode: overrides?.agentType ?? agent.agent_type ?? "host",
                    controller: isPersistent ? "persistent" : "subprocess",
                    cmd: cliBin,
                    "cmd:args": cliArgs,
                    "cmd:cwd": finalWorkDir,
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

            // Record this launch as an `AgentInstance` row in the DB so the
            // backend can track which pane is running which definition,
            // surface concurrent launches, and (later) carry GitHub work
            // context. Best-effort — a failure here doesn't abort the
            // launch (the agent already started). Stash the instance id
            // into block meta so downstream code (status updates,
            // bus targeting, lineage) can reference it. See
            // SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md §Phase 5.
            try {
                const inst = await RpcApi.CreateAgentInstanceCommand(TabRpcClient, {
                    definition_id: agent.id,
                    block_id: blockId,
                    // PR-F.3: launch modal carries the user's bundle
                    // picks. Empty / "blank" → backend resolver short-
                    // circuits so the agent inherits ambient creds.
                    identity_id: overrides?.identityId,
                    memory_id: overrides?.memoryId,
                });
                await RpcApi.SetMetaCommand(TabRpcClient, {
                    oref,
                    meta: { agentInstanceId: inst.id },
                });
            } catch (e: any) {
                Logger.warn(
                    "agent",
                    `agent instance row create failed: ${e?.message ?? String(e)}`,
                );
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
    agent?: ForgeAgent,
    instanceName?: string,
): AgentConfigFile[] {
    const files: AgentConfigFile[] = [];

    // Template variables for {{}} substitution. `AGENT` / `AGENT_DISPLAY`
    // prefer the resolved instance name so templates that reference
    // the agent identity (CLAUDE.md, skills) match what the shell
    // and MCP advertise for this run.
    const templateVars: Record<string, string> = {};
    if (agent) {
        const displayName = instanceName || agent.name;
        templateVars["AGENT"] = displayName;
        templateVars["AGENT_DISPLAY"] = displayName;
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

    // Always write .claude/settings.json with the auto-injected
    // PreToolUse:Bash hook (under the `hooks` key) so live streaming
    // engages on every session. User-supplied legacy hooks content
    // and user settings.json content both merge in. Mirror of
    // agentmux-srv/src/backend/agent_config.rs build_settings_with_hooks —
    // keep the two paths in sync.
    //
    // FILE LOCATION (v0.33.805+): Claude Code reads project hooks from
    // .claude/settings.json under the "hooks" key. A standalone
    // .claude/hooks.json is NOT a discovery location — that was the
    // v0.33.804 root cause: file was written but Claude never read it.
    // See https://code.claude.com/docs/en/hooks.md.
    const settingsJson = buildSettingsWithHooks(contentMap["settings"], contentMap["hooks"]);
    if (settingsJson) {
        files.push({ path: ".claude/settings.json", content: settingsJson });
    }

    // Build .mcp.json: auto-inject AgentMux MCP + merge user-provided config
    const mcpConfig = buildMcpConfig(contentMap["mcp"], agent, instanceName);
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
 * Build .claude/hooks.json content with the auto-injected PreToolUse:Bash
 * entry pointing at `agentmux-bashwrap hook`. User-supplied hooks merge
 * in: non-PreToolUse keys win on collision; user PreToolUse matchers
 * are appended BEFORE ours so a user deny-rule can short-circuit before
 * our rewrite fires.
 *
 * Mirror of `build_hooks_config` in
 * `agentmux-srv/src/backend/agent_config.rs`. The two paths must stay
 * in sync — keep changes aligned across both files. See
 * docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §5.
 */
function buildSettingsWithHooks(
    userSettingsContent: string | undefined,
    userHooksContent: string | undefined,
): string | null {
    const agentmuxPretooluse = {
        matcher: "^(Bash|.*[Bb]ash.*)$",
        hooks: [
            { type: "command", command: "agentmux-bashwrap hook" },
        ],
    };
    const hooksObj: Record<string, unknown> = {};
    const pretooluseEntries: unknown[] = [];

    if (userHooksContent) {
        let parsed: unknown;
        try {
            parsed = JSON.parse(userHooksContent);
        } catch (e) {
            console.warn("agent-model: failed to parse user hooks JSON; dropping", e);
            parsed = null;
        }
        if (parsed != null && typeof parsed === "object" && !Array.isArray(parsed)) {
            for (const [k, v] of Object.entries(parsed as Record<string, unknown>)) {
                if (k === "PreToolUse") {
                    if (Array.isArray(v)) {
                        pretooluseEntries.push(...v);
                    } else {
                        console.warn("agent-model: user hooks.PreToolUse is not an array; dropping");
                    }
                } else {
                    hooksObj[k] = v;
                }
            }
        } else if (parsed != null) {
            console.warn("agent-model: user hooks top-level is not an object; dropping");
        }
    }
    pretooluseEntries.push(agentmuxPretooluse);
    hooksObj["PreToolUse"] = pretooluseEntries;

    // Wrap into settings.json shape, merging any user-supplied settings.
    const settingsObj: Record<string, unknown> = {};
    if (userSettingsContent) {
        let parsed: unknown;
        try {
            parsed = JSON.parse(userSettingsContent);
        } catch (e) {
            console.warn("agent-model: failed to parse user settings JSON; dropping", e);
            parsed = null;
        }
        if (parsed != null && typeof parsed === "object" && !Array.isArray(parsed)) {
            Object.assign(settingsObj, parsed as Record<string, unknown>);
        } else if (parsed != null) {
            console.warn("agent-model: user settings top-level is not an object; dropping");
        }
    }
    // Merge existing user settings.hooks. For PreToolUse, user matchers are
    // PREPENDED so they short-circuit before our auto-injected entry. Other
    // event types (PostToolUse, Stop, etc.) pass through. Reagent P1 on
    // #813 caught the previous `continue` as a silent drop of user
    // PreToolUse from settings.json.
    const existingHooks = settingsObj["hooks"];
    if (existingHooks != null && typeof existingHooks === "object" && !Array.isArray(existingHooks)) {
        for (const [k, v] of Object.entries(existingHooks as Record<string, unknown>)) {
            if (k === "PreToolUse") {
                if (Array.isArray(v)) {
                    const ours = Array.isArray(hooksObj["PreToolUse"]) ? hooksObj["PreToolUse"] as unknown[] : [];
                    hooksObj["PreToolUse"] = [...v, ...ours];
                } else {
                    console.warn("agent-model: user settings.hooks.PreToolUse is not an array; dropped");
                }
                continue;
            }
            if (!(k in hooksObj)) hooksObj[k] = v;
        }
    }
    settingsObj["hooks"] = hooksObj;

    try {
        return JSON.stringify(settingsObj, null, 2);
    } catch (e) {
        console.error("agent-model: failed to serialize settings.json", e);
        return null;
    }
}

/**
 * Build .mcp.json content with auto-injected AgentMux MCP server.
 * Merges with user-provided MCP config if present.
 */
function buildMcpConfig(
    userMcpContent: string | undefined,
    agent?: ForgeAgent,
    instanceName?: string,
): string | null {
    // Auto-inject AgentMux MCP server for inter-agent messaging.
    // `AGENTMUX_AGENT_ID` must match the shell's / pane title's
    // `AGENTMUX_AGENT_ID` (the resolved instance name), otherwise
    // inter-agent routing targets the definition name while
    // everything else advertises the instance name.
    const agentMuxServer: Record<string, any> = {
        type: "stdio",
        command: "agentmux-mcp",
        args: [],
        env: {} as Record<string, string>,
    };
    if (agent) {
        agentMuxServer.env["AGENTMUX_AGENT_ID"] = instanceName || agent.name;
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
