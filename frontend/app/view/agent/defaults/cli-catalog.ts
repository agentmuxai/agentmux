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
 * - `AgentCard` display (title = displayName, caption = blurb)
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
    mcpSupport: "stdio" | "stdio+http" | "stdio+http+oauth" | "none";
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
        containerImage: "ghcr.io/agentmuxai/agent-base:latest",
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
        // Since 2026-06-18 Gemini CLI no longer serves personal Google
        // accounts (free, AI Pro, Ultra); Antigravity replaced it for those.
        blurb: "Google's coding agent, with a Gemini API key",
        primaryContextFile: "GEMINI.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Google's coding agent. Very large context window — it can look at a lot of code at once. Defaults to reading + planning before it writes anything, so it's a safer pick when you're still deciding what to change. Needs a paid Gemini API key or a company Gemini Code Assist licence: since June 2026 it no longer works with a personal Google account. For that, pick Antigravity.",
        hostSupported: true,
        // No container image built yet — host-only until agentmux/gemini ships.
        containerSupported: false,
    },
    {
        provider: "antigravity",
        displayName: "Antigravity (AGY)",
        icon: "⚛",
        blurb: "Google's agentic coding CLI harness",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Google's agentic coding CLI harness, which replaced Gemini CLI for personal Google accounts in June 2026. Emphasizes high-throughput agentic task execution with native skill discovery and subagent support. Uses your Google account. Installed with Google's own installer, not by AgentMux.",
        hostSupported: true,
        // No container image built yet — host-only until agentmux/antigravity ships.
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
        blurb: "Minimal multi-provider coding agent",
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
    {
        provider: "kilo",
        displayName: "Kilo Code CLI",
        icon: "K",
        blurb: "Open-source agent with 500+ models",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "An open-source coding agent with access to hundreds of models through Kilo's gateway, including free ones, so it runs before you sign in. Sign in with `kilo auth login` for your own account or keys.",
        hostSupported: true,
        // ACP runs on the host only (no container ACP path yet).
        containerSupported: false,
    },
    {
        provider: "cline",
        displayName: "Cline CLI",
        icon: "C",
        blurb: "Plan-then-act agent, any model",
        primaryContextFile: "AGENTS.md",
        // Every ACP agent takes stdio MCP servers; Cline advertises no http or sse.
        mcpSupport: "stdio",
        popoverMarkdown:
            "Cline's coding agent from the command line. It plans before it acts and works with most model providers. It needs a sign-in first: run `cline auth` (a Cline account, a ChatGPT subscription or your own API key).",
        hostSupported: true,
        // ACP runs on the host only (no container ACP path yet).
        containerSupported: false,
    },
    {
        provider: "opencode",
        displayName: "OpenCode",
        icon: "O",
        blurb: "Open-source agent, 75+ providers",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "The most-used open-source coding agent. Works with 75+ model providers and comes with free models, so it runs before you sign in. Sign in with `opencode auth login` to use your own providers.",
        hostSupported: true,
        // ACP runs on the host only (no container ACP path yet).
        containerSupported: false,
    },
    {
        provider: "goose",
        displayName: "goose",
        icon: "G",
        blurb: "Open-source agent from the Linux Foundation",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "An open-source, extensible agent (Agentic AI Foundation, Linux Foundation) that works with 15+ model providers. Install it with goose's own installer, then run `goose configure` once to pick a provider and key.",
        hostSupported: true,
        // ACP runs on the host only (no container ACP path yet).
        containerSupported: false,
    },
    {
        provider: "grok",
        displayName: "Grok Build",
        icon: "X",
        blurb: "xAI's coding agent",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "xAI's coding agent, running Grok 4.6 by default. Install it with xAI's installer, then sign in with `grok login` (a SuperGrok or X Premium+ account) or set XAI_API_KEY.",
        hostSupported: true,
        // ACP runs on the host only (no container ACP path yet).
        containerSupported: false,
    },
    {
        provider: "dsh",
        displayName: "DeepSeek Harness",
        icon: "D",
        blurb: "DeepSeek's open-source harness (preview)",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "DeepSeek's open-source agent harness. A preview: upstream ships prereleases and promises breaking changes. Set a DeepSeek API key in its web UI (`dsh web`) or as DEEPSEEK_API_KEY. AgentMux turns off its session-log upload to DeepSeek.",
        hostSupported: true,
        // ACP runs on the host only (no container ACP path yet).
        containerSupported: false,
    },
    {
        provider: "aider",
        displayName: "Aider",
        icon: "A",
        blurb: "Git-native pair programmer",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "none",
        popoverMarkdown:
            "Aider, the git-native pair programmer, in its own terminal UI. It commits its changes as it goes. Install it with Aider's installer and set your model provider's key (for example OPENAI_API_KEY or ANTHROPIC_API_KEY). It opens in a terminal pane; AgentMux shows no transcript of its own.",
        hostSupported: true,
        // A terminal pane on the host.
        containerSupported: false,
    },
    {
        provider: "crush",
        displayName: "Crush",
        icon: "C",
        blurb: "Charm's terminal coding agent",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Charm's coding agent, in its own terminal UI. Works with many model providers; sign in with `crush login` or set a key in its UI. It opens in a terminal pane; AgentMux shows no transcript of its own. Note: Crush adds an Assisted-by line to commits unless its config turns that off.",
        hostSupported: true,
        // A terminal pane on the host.
        containerSupported: false,
    },
    {
        provider: "muse",
        displayName: "Muse Code",
        icon: "M",
        blurb: "Meta's coding agent",
        primaryContextFile: "AGENTS.md",
        mcpSupport: "stdio+http",
        popoverMarkdown:
            "Meta's coding agent, running Muse Spark, in its own terminal UI. Install it with Meta's installer and sign in with `muse login` or META_API_KEY. A Muse subscription runs in contributor mode by default, which lets Meta train on your content; change it in Muse's settings. It opens in a terminal pane for now.",
        hostSupported: true,
        // A terminal pane on the host.
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
