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
    /** Value stored in AgentDefinition.provider. */
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

const CLI_CATALOG: CliCatalogEntry[] = [
    {
        provider: "claude",
        displayName: "Claude Code",
        icon: "✖",
        blurb: "Anthropic's coding agent",
        primaryContextFile: "CLAUDE.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Anthropic's coding agent. Strong at reasoning through long sessions and explaining its thinking as it works. Good when you want it to read widely across a codebase before changing anything. Best if you already use Claude.",
        hostSupported: true,
        containerSupported: true,
        containerImage: "ghcr.io/agentmuxai/agent-claude:latest",
    },
    {
        provider: "codex",
        displayName: "Codex CLI",
        icon: "✦",
        blurb: "OpenAI's coding agent",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http+oauth",
        popoverMarkdown:
            "OpenAI's coding agent. Fast and focused — good at small, well-defined tasks and quick refactors. Can work on several files in parallel. Uses your OpenAI account.",
        hostSupported: true,
        // No container image built yet — host-only until agentmux/codex ships.
        containerSupported: false,
    },
    {
        provider: "gemini",
        displayName: "Gemini CLI",
        icon: "⚡",
        blurb: "Google's coding agent",
        primaryContextFile: "GEMINI.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Google's coding agent. Very large context window — it can look at a lot of code at once. Defaults to reading + planning before it writes anything, so it's a safer pick when you're still deciding what to change. Uses your Google account.",
        hostSupported: true,
        // No container image built yet — host-only until agentmux/gemini ships.
        containerSupported: false,
    },
    {
        provider: "qwen",
        displayName: "Qwen Code",
        icon: "❖",
        blurb: "Alibaba's open-source coding agent",
        primaryContextFile: "QWEN.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Alibaba's open-source coding agent (a fork of Gemini CLI) tuned for the Qwen3-Coder models. Runs on any OpenAI-compatible endpoint — point it at OpenRouter and use a huge range of models with a single key. Good when you want broad open-model coverage or a low-cost backend.",
        hostSupported: true,
        // No agentmux/qwen container image is built yet — host-only for now.
        // Flip to true + add containerImage once the image ships.
        containerSupported: false,
    },
    {
        provider: "kimi",
        displayName: "Kimi Code",
        icon: "◈",
        blurb: "Moonshot's 262k-context agent",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "An open-source coding agent from Moonshot with a huge memory — it can hold a lot of project history in mind at once. Good when you need it to remember a long conversation or read many files without losing track.",
        hostSupported: true,
        // No container image built yet — host-only until agentmux/kimi ships.
        containerSupported: false,
    },
    {
        provider: "pi",
        displayName: "Pi",
        icon: "π",
        blurb: "Plandex's multi-provider agent",
        primaryContextFile: "AGENTS.md or CLAUDE.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "A flexible agent that can run on your choice of model — Claude, OpenAI, Google, and more. Pick this if you want to try different models against the same task, or if you have API keys from several providers and want to switch easily.",
        hostSupported: true,
        // No container image built yet — host-only until agentmux/pi ships.
        containerSupported: false,
    },
    {
        provider: "openclaw",
        displayName: "OpenClaw",
        icon: "◎",
        blurb: "ACP orchestration platform",
        primaryContextFile: "AGENTS.md + SOUL.md + …",
        mcpSupport: "none",
        popoverMarkdown:
            "Not a coding agent itself — it runs *other* agents for you and keeps them coordinated. Pick this when you want several agents working in parallel and handing tasks to each other, rather than doing the work directly.",
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
            "GitHub's coding agent. Integrates closely with GitHub repos and pull requests. Has a switch between Interactive, Plan, and Autopilot so you can choose how much freedom it gets. Best if you mostly work on GitHub and have a Copilot subscription.",
        hostSupported: true,
        // No container image built yet — host-only until agentmux/copilot ships.
        containerSupported: false,
    },
];

/**
 * Look up a catalog entry by provider id. Returns null when the
 * provider is unknown (e.g. a user-defined provider that predates
 * the catalog). Callers should render fallback values from the
 * AgentDefinition row in that case.
 */
export function getCliCatalogEntry(provider: string): CliCatalogEntry | null {
    const lower = (provider || "").toLowerCase();
    return CLI_CATALOG.find((c) => c.provider === lower) ?? null;
}
