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
 * `skill_type` value that materializes a skill as an Agent Skills-format
 * `.claude/skills/<slug>/SKILL.md` instead of a `.claude/commands/<trigger>.md`
 * slash command. Mirrors `SKILL_TYPE_AGENT_SKILL` in
 * `agentmux-srv/src/backend/agent_config.rs` — keep the two in sync.
 */
const SKILL_TYPE_AGENT_SKILL = "agent-skill";

/**
 * Build the list of config files to write to the agent working directory.
 * Assembles CLAUDE.md from soul + agentmd + memory + skills index,
 * writes each skill as a slash command in .claude/commands/ (or an Agent
 * Skills-format SKILL.md under .claude/skills/, for skill_type ===
 * SKILL_TYPE_AGENT_SKILL), writes hooks.json if present, auto-injects
 * AgentMux MCP server, and applies template variable substitution.
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

    // Write each skill as either a slash command (.claude/commands/{trigger}.md,
    // default) or an Agent Skills-format SKILL.md (.claude/skills/{slug}/SKILL.md,
    // skill_type === SKILL_TYPE_AGENT_SKILL).
    const usedSkillSlugs = new Set<string>();
    for (const skill of skills) {
        if (!skill.content) continue;
        if (skill.skill_type === SKILL_TYPE_AGENT_SKILL) {
            const slug = uniqueSkillSlug(skill.name, usedSkillSlugs);
            const content = expandTemplate(skill.content, templateVars);
            files.push({
                path: `.claude/skills/${slug}/SKILL.md`,
                content: renderSkillMd(slug, skill.description, content),
            });
        } else {
            const safeTrigger = sanitizeTrigger(skill.trigger);
            if (safeTrigger) {
                const content = expandTemplate(skill.content, templateVars);
                files.push({ path: `.claude/commands/${safeTrigger}.md`, content });
            }
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
function expandTemplate(content: string, vars: Record<string, string>): string {
    return content.replace(/\{\{(\w+)\}\}/g, (match, key) => {
        return vars[key] ?? match;
    });
}

/**
 * Derive a filesystem-safe slug from a display name. Lowercase, ASCII
 * alphanumeric + dash/underscore, consecutive dashes collapsed, trimmed to
 * 64 chars. Falls back to "agent" if the input has no valid characters.
 *
 * Mirrors `derive_slug` in `agentmux-srv/src/backend/storage/agents.rs` —
 * keep the two in sync, or a SKILL.md preview built here won't match the
 * path the authoritative Rust launch path actually writes to.
 */
export function deriveSlug(name: string): string {
    const filtered = name
        .toLowerCase()
        .replace(/[^a-z0-9-_]/g, "-");
    const collapsed = filtered
        .split("-")
        .filter((s) => s.length > 0)
        .join("-");
    const trimmed = collapsed.slice(0, 64);
    return trimmed || "agent";
}

/** Agent Skills spec caps `description` at 1024 characters. */
const SKILL_DESCRIPTION_MAX_LEN = 1024;

/**
 * Render an Agent Skills-format `SKILL.md`: YAML frontmatter with the two
 * required fields (`name`, `description`), followed by the skill's content
 * as the Markdown body. See https://agentskills.io/specification.
 *
 * `slug` (not the skill's raw display name) is REQUIRED here — the spec
 * requires `name` be lowercase/hyphenated and match its parent directory;
 * callers must pass the same value used to build the `.claude/skills/<slug>/`
 * path (reagent P1, PR #2322). `description` is validated: the spec requires
 * a non-empty value (falls back to a placeholder) capped at 1024 characters.
 *
 * YAML double-quoted scalars use JSON-compatible escaping (YAML 1.2
 * §7.3.1), so `JSON.stringify` on a plain string produces a valid,
 * correctly-escaped YAML value — same reasoning as `render_skill_md` in
 * `agentmux-srv/src/backend/agent_config.rs`, which this mirrors.
 */
export function renderSkillMd(slug: string, description: string, body: string): string {
    const desc = description.trim()
        ? description.slice(0, SKILL_DESCRIPTION_MAX_LEN)
        : "No description provided.";
    return `---\nname: ${JSON.stringify(slug)}\ndescription: ${JSON.stringify(desc)}\n---\n\n${body}`;
}

/**
 * Validate a skill's `trigger` is safe to use as a single path segment in
 * `.claude/commands/<trigger>.md`. `trigger` is free-form user input with no
 * format validation anywhere upstream, so a trigger containing a path
 * separator or a `..` segment previously let the resulting filename resolve
 * OUTSIDE the agent's working directory (reagent P1, PR #2322). Returns
 * `null` for anything containing "/" or "\\", or that is exactly "."/"..";
 * callers skip writing that skill's command file entirely. Mirrors
 * `sanitize_trigger` in `agentmux-srv/src/backend/agent_config.rs`.
 */
export function sanitizeTrigger(trigger: string): string | null {
    if (!trigger || trigger === "." || trigger === "..") {
        return null;
    }
    if (trigger.includes("/") || trigger.includes("\\")) {
        return null;
    }
    return trigger;
}

/**
 * Derive a slug for an Agent Skill name that is valid per the Agent Skills
 * `name` grammar: lowercase letters, digits, and hyphens ONLY (no
 * underscores). `deriveSlug` is shared with agent role-slugs, which
 * deliberately DO permit underscores, so it isn't spec-valid here as-is —
 * hyphenate underscores (and re-collapse any resulting run of hyphens)
 * rather than reusing it directly (Codex P1, PR #2322). Mirrors
 * `skill_name_slug` in `agentmux-srv/src/backend/agent_config.rs`.
 */
function skillNameSlug(name: string): string {
    const collapsed = deriveSlug(name)
        .replace(/_/g, "-")
        .split("-")
        .filter((s) => s.length > 0)
        .join("-");
    return collapsed || "skill";
}

/**
 * Derive a filesystem-safe, COLLISION-FREE, spec-valid slug for a skill
 * within one `buildConfigFiles` call. `skillNameSlug` alone can produce
 * identical output for distinct names that differ only in
 * punctuation/whitespace (e.g. "Deploy Checklist" and "Deploy!!!Checklist"
 * both -> "deploy-checklist"), which would otherwise silently overwrite one
 * skill's SKILL.md with another's (reagent P1, PR #2322). Appends "-2",
 * "-3", ... until unique within `used`, truncating the base first so the
 * suffixed result never exceeds the spec's 64-character max (Codex P2, PR
 * #2322). Mirrors `unique_skill_slug` in
 * `agentmux-srv/src/backend/agent_config.rs`.
 */
export function uniqueSkillSlug(name: string, used: Set<string>): string {
    const MAX_LEN = 64;
    const base = skillNameSlug(name);
    if (!used.has(base)) {
        used.add(base);
        return base;
    }
    let n = 2;
    for (;;) {
        const suffix = `-${n}`;
        const truncatedBase = base.slice(0, Math.max(0, MAX_LEN - suffix.length));
        const candidate = `${truncatedBase}${suffix}`;
        if (!used.has(candidate)) {
            used.add(candidate);
            return candidate;
        }
        n++;
    }
}

/**
 * Merge a user-supplied `settings.json`-level hook-array (`PreToolUse`
 * or `PreCompact`) with whatever is already staged in `hooksObj` under
 * `key` (AgentMux's own auto-injected entries, possibly already
 * carrying legacy `content_map["hooks"]`-merged user entries from the
 * earlier pass). User entries are PREPENDED so their matchers/gates
 * get first refusal; AgentMux's own entries always stay last.
 *
 * Mirror of `prepend_user_hook_array` in
 * `agentmux-srv/src/backend/agent_config.rs` — keep the two in sync.
 */
function prependUserHookArray(hooksObj: Record<string, unknown>, key: string, userValue: unknown): void {
    if (!Array.isArray(userValue)) {
        console.warn(`agent-model: user settings.hooks.${key} is not an array; dropped`);
        return;
    }
    const ours = Array.isArray(hooksObj[key]) ? hooksObj[key] as unknown[] : [];
    hooksObj[key] = [...userValue, ...ours];
}

/**
 * Build .claude/hooks.json content with the auto-injected PreToolUse:Bash
 * entry pointing at `agentmux-bashwrap hook`, plus two PreCompact entries
 * (matcher "manual" / "auto") pointing at `agentmux-bashwrap precompact`
 * so a live "compaction started" signal reaches the sidecar (see
 * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.2).
 * User-supplied hooks merge in: non-PreToolUse/PreCompact keys win on
 * collision; user PreToolUse/PreCompact matchers are appended BEFORE
 * ours so a user deny-rule (or custom PreCompact observer) can
 * short-circuit before our rewrite/observation fires.
 *
 * Mirror of `build_settings_with_hooks` in
 * `agentmux-srv/src/backend/agent_config.rs`. The two paths must stay
 * in sync — keep changes aligned across both files (Codex P1, PR #2378:
 * this mirror originally lagged the Rust builder by one hook type,
 * so agents launched through the standard picker never got PreCompact
 * installed). See docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §5.
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
    // `PreCompact` requires an explicit `matcher` ("manual" or "auto") —
    // Claude Code has no confirmed wildcard-all value for this hook —
    // so two separate entries are registered, each with a different
    // static `--trigger=` argv baked in so the binary knows which fired
    // without needing it from stdin (PreCompact's stdin payload carries
    // no `trigger` field; see `agentmux-bashwrap/src/precompact.rs`).
    const agentmuxPrecompactManual = {
        matcher: "manual",
        hooks: [
            { type: "command", command: "agentmux-bashwrap precompact --trigger=manual" },
        ],
    };
    const agentmuxPrecompactAuto = {
        matcher: "auto",
        hooks: [
            { type: "command", command: "agentmux-bashwrap precompact --trigger=auto" },
        ],
    };
    const hooksObj: Record<string, unknown> = {};
    const pretooluseEntries: unknown[] = [];
    const precompactEntries: unknown[] = [];

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
                } else if (k === "PreCompact") {
                    if (Array.isArray(v)) {
                        precompactEntries.push(...v);
                    } else {
                        console.warn("agent-model: user hooks.PreCompact is not an array; dropping");
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
    precompactEntries.push(agentmuxPrecompactManual, agentmuxPrecompactAuto);
    hooksObj["PreCompact"] = precompactEntries;

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
    // Merge existing user settings.hooks. For PreToolUse and PreCompact,
    // user matchers are PREPENDED so they short-circuit before our
    // auto-injected entries. Other event types (PostToolUse, Stop, etc.)
    // pass through. Reagent P1 on #813 caught the previous `continue` as
    // a silent drop of user PreToolUse from settings.json; PreCompact is
    // deliberately folded into the same array-merge discipline rather
    // than the generic `if (!(k in hooksObj))` path below, or a
    // user-supplied PreCompact entry would hit that path and be
    // silently and permanently dropped the moment PreCompact became
    // auto-injected too (Codex P1, PR #2378).
    const existingHooks = settingsObj["hooks"];
    if (existingHooks != null && typeof existingHooks === "object" && !Array.isArray(existingHooks)) {
        for (const [k, v] of Object.entries(existingHooks as Record<string, unknown>)) {
            if (k === "PreToolUse" || k === "PreCompact") {
                prependUserHookArray(hooksObj, k, v);
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
