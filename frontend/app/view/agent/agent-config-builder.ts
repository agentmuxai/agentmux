// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure config-synthesis helpers extracted from agent-model.ts (see
 * docs/specs — modularization pass, 2026-07-23). These build the
 * CLAUDE.md / .claude/settings.json / .mcp.json content written into a
 * freshly-launched agent's working directory.
 *
 * Pure functions only: no `this`, no RpcApi calls, no SolidJS. Callers
 * (agent-model.ts's `launchAgentDefinition`) own all I/O.
 */

import { Logger } from "@/util/logger";

/**
 * Build the list of config files to write to the agent working directory.
 * Assembles CLAUDE.md from soul + agentmd + memory + skills index,
 * writes each skill as a slash command in .claude/commands/,
 * writes hooks.json if present, auto-injects AgentMux MCP server,
 * and applies template variable substitution.
 */
export function buildConfigFiles(
    contentMap: Record<string, string>,
    skills: AgentSkill[] = [],
    agent?: AgentDefinition,
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
            const descPart = skill.description ? ` — ${skill.description}` : "";
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
    const agentSlug = agent ? (agent.slug || agent.name.toLowerCase().replace(/[^a-z0-9-_]/g, "-")) : undefined;
    const mcpConfig = buildMcpConfig(contentMap["mcp"], agent, instanceName, agentSlug);
    if (mcpConfig) {
        files.push({ path: ".mcp.json", content: mcpConfig });
    }

    return files;
}

/**
 * Replace {{VARIABLE}} placeholders in content with values from vars map.
 */
export function expandTemplate(content: string, vars: Record<string, string>): string {
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
export function buildSettingsWithHooks(
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
export function buildMcpConfig(
    userMcpContent: string | undefined,
    agent?: AgentDefinition,
    instanceName?: string,
    slug?: string,
): string | null {
    // Auto-inject AgentMux MCP server for inter-agent messaging.
    // `AGENTMUX_AGENT_ID` must be the stable role slug (matching the
    // pane env var) so agentmux-mcp advertises the same routing ID
    // whether the agent reads it from its environment or from
    // the MCP tool source field.
    const agentMuxServer: Record<string, any> = {
        type: "stdio",
        command: "agentmux-mcp",
        args: [],
        env: {} as Record<string, string>,
    };
    if (agent) {
        agentMuxServer.env["AGENTMUX_AGENT_ID"] = slug || instanceName || agent.name;
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
            Logger.error("agent", "Invalid MCP JSON in agent content, using auto-injected only");
        }
    }

    return JSON.stringify(mcpObj, null, 2);
}
