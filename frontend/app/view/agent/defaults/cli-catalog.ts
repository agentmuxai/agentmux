// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CLI catalog — the single source of truth for agent-pane definition
 * cards. Seeded from the research in GitHub discussion #493:
 *   https://github.com/agentmuxai/agentmux/discussions/493
 *
 * Each entry describes what a CLI *does* — the agent-pane card leads
 * with that (capability-first) and keeps the CLI brand name as a
 * secondary caption. The `popoverMarkdown` field is shown when the
 * user hovers the card's ⓘ affordance and quotes the per-CLI
 * startup / context / memory behaviour from the research doc.
 *
 * Drives:
 * - `AgentCard` display (title = blurb, caption = displayName)
 * - `AgentLaunchModal` description text
 * - `AgentLaunchModal` default container image when "Container" is
 *   picked
 *
 * When discussion #493 is updated with new CLIs or new facts, this
 * file must be updated in lockstep; the two are versioned together.
 */

export interface CliCatalogEntry {
    /** Value stored in ForgeAgent.provider. */
    provider: string;
    /** CLI brand name shown as the card's secondary caption. */
    displayName: string;
    /** Unicode glyph for the card's big left-side icon. */
    icon: string;
    /** Capability-first one-liner used as the card title. */
    blurb: string;
    /** Primary context file the CLI reads at startup — shown as a badge. */
    primaryContextFile: string;
    /** Short MCP-support summary; "none" hides the badge. */
    mcpSupport: "stdio+http" | "stdio+http+oauth" | "none";
    /** Long description shown in the card's ⓘ popover. Markdown-ish
     *  (one paragraph, short lines). */
    popoverMarkdown: string;
    /** Can this CLI be launched directly on the host? */
    hostSupported: boolean;
    /** Can this CLI be launched inside a container? */
    containerSupported: boolean;
    /** Default container image tag when containerSupported is true. */
    containerImage?: string;
}

export const CLI_CATALOG: CliCatalogEntry[] = [
    {
        provider: "claude",
        displayName: "Claude Code",
        icon: "✖",
        blurb: "Anthropic's coding agent",
        primaryContextFile: "CLAUDE.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Walks CWD → root loading CLAUDE.md + CLAUDE.local.md at each level. Merges ~/.claude/CLAUDE.md and org policy. Reads .claude/rules/**. MCP over stdio + HTTP. Auto-memory in ~/.claude/projects/<hash>/memory/. No size limit (guideline: keep CLAUDE.md under 200 lines).",
        hostSupported: true,
        containerSupported: true,
        containerImage: "agentmux/claude:latest",
    },
    {
        provider: "codex",
        displayName: "Codex CLI",
        icon: "✦",
        blurb: "OpenAI's coding agent",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http+oauth",
        popoverMarkdown:
            "Loads AGENTS.md: global → git root → CWD, concatenated (32KB limit). Config is TOML at ~/.codex/config.toml. 6 concurrent agent threads by default. MCP over stdio, HTTP, OAuth. Enterprise-enforced requirements.toml supported.",
        hostSupported: true,
        containerSupported: true,
        containerImage: "agentmux/codex:latest",
    },
    {
        provider: "gemini",
        displayName: "Gemini CLI",
        icon: "⚡",
        blurb: "Google's coding agent",
        primaryContextFile: "GEMINI.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Loads ~/.gemini/GEMINI.md + ./GEMINI.md (walks up to .git). Subdirectory GEMINI.md files load just-in-time (v0.34.0+). Plan Mode is default — read-only until user approves writes. GEMINI_SYSTEM_MD replaces the system prompt entirely.",
        hostSupported: true,
        containerSupported: true,
        containerImage: "agentmux/gemini:latest",
    },
    {
        provider: "kimi",
        displayName: "Kimi Code",
        icon: "◈",
        blurb: "Moonshot's 262k-context agent",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Discovers AGENTS.md in project root (auto-generates via /init if absent). Config TOML at ~/.kimi/. MCP at ~/.kimi/mcp.json. Injects KIMI_* system-prompt vars (KIMI_NOW, KIMI_WORK_DIR, KIMI_AGENTS_MD, etc.). Context window: 262,144 tokens (K2.6).",
        hostSupported: true,
        containerSupported: true,
        containerImage: "agentmux/kimi:latest",
    },
    {
        provider: "pi",
        displayName: "Pi",
        icon: "π",
        blurb: "Plandex's multi-provider agent",
        primaryContextFile: "AGENTS.md or CLAUDE.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Reads ~/.pi/agent/AGENTS.md (or CLAUDE.md) globally plus .pi/AGENTS.md walked from CWD. 4 built-in tools (Read, Write, Edit, Bash) — everything else via extensions. 15+ model providers. Supports ACP so another agent can orchestrate it. Disable context with -nc.",
        hostSupported: true,
        containerSupported: true,
        containerImage: "agentmux/pi:latest",
    },
    {
        provider: "openclaw",
        displayName: "OpenClaw",
        icon: "◎",
        blurb: "ACP orchestration platform",
        primaryContextFile: "AGENTS.md + SOUL.md + …",
        mcpSupport: "none",
        popoverMarkdown:
            "OpenClaw is an orchestrator, not a coding agent itself. Manages N child agents (Pi, Claude, Codex, Gemini) over ACP (JSON-RPC 2.0 / stdio). First-turn injects AGENTS.md + SOUL.md + TOOLS.md + IDENTITY.md + USER.md + HEARTBEAT.md + MEMORY.md + BOOTSTRAP.md. Single Gateway daemon with lane-aware FIFO queue.",
        hostSupported: true,
        containerSupported: false,
    },
    {
        provider: "copilot",
        displayName: "GitHub Copilot CLI",
        icon: "⛶",
        blurb: "Microsoft's coding agent",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Reads AGENTS.md + .github/copilot-instructions.md + .github/instructions/**/*.instructions.md. Also reads CLAUDE.md and GEMINI.md at repo root as compat. Config JSONC at ~/.copilot/config.json. Shift+Tab cycles Interactive → Plan → Autopilot. Built-in subagents: Explore / Task / Plan / Code-review. MCP user-scope only today.",
        hostSupported: true,
        containerSupported: true,
        containerImage: "agentmux/copilot:latest",
    },
];

/**
 * Look up a catalog entry by provider id. Returns null when the
 * provider is unknown (e.g. a user-defined provider that predates
 * the catalog). Callers should render fallback values from the
 * ForgeAgent row in that case.
 */
export function getCliCatalogEntry(provider: string): CliCatalogEntry | null {
    const lower = (provider || "").toLowerCase();
    return CLI_CATALOG.find((c) => c.provider === lower) ?? null;
}
