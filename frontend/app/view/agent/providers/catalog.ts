// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ProviderDefinition, SystemPrereq } from "./types";

/** Shared git prereq — claude-code calls `git` from session-start
 *  (issue anthropics/claude-code#29898), and openclaw uses git for
 *  project context. Curated install URLs per platform. */
export const GIT_PREREQ: SystemPrereq = {
    tool: "git",
    label: "Git",
    installUrls: {
        windows: "https://git-scm.com/download/win",
        macos: "https://git-scm.com/download/mac",
        linux: "https://git-scm.com/download/linux",
    },
    installLinkText: {
        windows: "Install Git for Windows",
        macos: "Install Git for macOS",
        linux: "Install Git",
    },
};

export const PROVIDERS: Record<string, ProviderDefinition> = {
    claude: {
        id: "claude",
        displayName: "Claude Code",
        cliCommand: "claude",
        defaultArgs: [],
        styledArgs: ["--output-format", "stream-json", "--verbose", "--include-partial-messages", "--dangerously-skip-permissions"],
        outputFormat: "raw",
        styledOutputFormat: "claude-stream-json",
        authType: "oauth",
        authCheckCommand: ["auth", "status", "--json"],
        // NOTE (2026-08-03, SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §2): in-app
        // OAuth is VIABLE again for Claude — this supersedes the 2026-06-23
        // "DEAD END for Claude v2.1.x" verdict (SPEC_HOST_CLI_LOGIN_CAPTURE §0),
        // whose factual basis (v2.1.183 never printed a login URL when
        // host-spawned) is stale. Live probes on 2026-08-03 against the pinned
        // CLI (2.1.198) and a current global install (2.1.214) confirmed that
        // `claude auth login`, spawned under a PTY with an isolated
        // CLAUDE_CONFIG_DIR and no DISPLAY, prints the full PKCE authorize URL
        // ("If the browser didn't open, visit: https://claude.com/cai/oauth/
        // authorize?…"), then prompts `Paste code here if prompted >` on stdin —
        // and if the user authorizes in a browser, the CLI detects completion
        // on its own (polling keyed by `state`) and exits successfully with no
        // paste needed. So tier 1 of runProviderLogin (URL capture + the
        // AuthUrlBox paste UI, via the existing set_provider_auth stdin
        // plumbing) works end to end; older CLIs that print nothing are handled
        // by the behavior-gate fallthrough to tiers 2/3 (spec §3.2), not a
        // version check. This is also the login argv the in-app session spawns
        // (spec §3.2's "login argv" — it was already here as the auth metadata).
        authLoginCommand: ["auth", "login"],
        // Run `claude auth login` under a PTY (run_cli_login's PTY branch).
        // Spawned with plain pipes from the GUI host, the CLI exits cleanly
        // ~5s after printing the OAuth URL — before the user can return from
        // the browser and paste the code — so the login always appeared stuck
        // on the URL. (Reproduced extensively: standalone runs with a
        // controlling terminal stay alive at the "Paste code here >" prompt;
        // only the host's terminal-less pipe spawn exits early.) A PTY makes
        // the CLI fully interactive (isTTY + a controlling terminal), so it
        // stays alive for the paste. See docs / run_cli_login_pty.
        requiresLoginTty: true,
        // `headlessLoginUrlUnsupported` was dropped here on 2026-08-03
        // (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.2): the pinned CLI
        // (2.1.198+) DOES print a scrapeable authorize URL under our PTY spawn
        // (see the probe note above), so tier 1's URL capture is live again for
        // Claude. The flag itself remains in ProviderDefinition as a
        // behavior-gate for providers whose CLI genuinely never prints one.
        npmPackage: "@anthropic-ai/claude-code",
        // Keep in sync with agentmux-srv/src/backend/providers.rs `pinned_version`,
        // agentmux-cef/src/commands/providers.rs `CLAUDE_VERSION`, and
        // .github/workflows/container-image.yml `claude_version` default — enforced by
        // ./pin-consistency.test.ts.
        pinnedVersion: "2.1.198",
        docsUrl: "https://docs.anthropic.com/claude-code",
        windowsInstallCommand: "irm https://claude.ai/install.ps1 | iex",
        unixInstallCommand: "curl -fsSL https://claude.ai/install.sh | bash",
        icon: "sparkles",
        unsetEnv: ["CLAUDECODE"],
        authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
        authDirName: "claude",
        // Documented Claude Code behavior: redirects the CLI at a non-Anthropic
        // (or proxied) backend — Bedrock, Vertex, OpenRouter, a custom proxy.
        // Mirrors agentmux-srv/src/backend/providers.rs `base_url_env_var`.
        baseUrlEnvVar: "ANTHROPIC_BASE_URL",
        supportedVendors: ["anthropic"],
        startupInstructionsFilename: "CLAUDE.md",
        launchArgs: ["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--dangerously-skip-permissions"],
        resumeFlag: "--resume",
        sessionIdField: "session_id",
        // Persistent (bidirectional stream-json) + the Agent SDK CONTROL PROTOCOL
        // is the only way AskUserQuestion works headless: the CLI auto-rejects it
        // ("Error: Answer questions?") unless launched with `--permission-prompt-tool
        // stdio` and answered via a control_response. `--dangerously-skip-permissions`
        // DISABLES that routing, so it must NOT be here. The persistent controller's
        // ControlChannel auto-allows ordinary tools to preserve today's yolo UX and
        // surfaces only AskUserQuestion to the user. Keep in sync with `static CLAUDE`
        // in agentmux-srv/providers.rs. controllerType selects persistentLaunchArgs
        // over launchArgs in useAgentCommands.ts.
        // Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
        controllerType: "persistent",
        persistentLaunchArgs: ["--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--permission-prompt-tool", "stdio", "--permission-mode", "default"],
        // Claude Code calls `git` at session-start (issue
        // anthropics/claude-code#29898). Without git the CLI fails
        // with `Error: Git is required but was not found.`.
        systemPrereqs: [GIT_PREREQ],
        contextWindow: 200_000,
        // Labels carry the concrete version the pinned CLI (see `pinnedVersion`)
        // currently resolves each family alias to — curated, kept in sync on a
        // pin bump (SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG Part B). The `value`
        // stays the family alias (opus/sonnet/haiku), which `--model` resolves
        // to exactly that version; when a family ever has TWO live versions,
        // switch those entries to concrete `--model` IDs (e.g. claude-sonnet-4-6).
        // `default: true` marks the family the runtime falls back to when no
        // model is set — it MUST match DEFAULT_RUNTIME_CONFIG.model (types.ts),
        // which is `sonnet` (faster default for routine turns). A mismatch here
        // desyncs the strip's Model dropdown from the actual default and is a
        // trap for any `models.find(m => m.default)` reader.
        models: [
            { value: "opus", label: "Opus 4.8", description: "Claude Opus 4.8 — highest quality", aliases: ["claude-opus"] },
            { value: "sonnet", label: "Sonnet 5", default: true, description: "Claude Sonnet 5 — balanced", aliases: ["claude-sonnet"] },
            { value: "haiku", label: "Haiku 4.5", description: "Claude Haiku 4.5 — fastest", aliases: ["claude-haiku"] },
            // No confirmed generic "fable" alias (unlike opus/sonnet/haiku above), so this
            // pins the concrete model id directly — same id already relied on by
            // context-window.ts's 1M-context-window band. Kept current here so the
            // offline/API-failure fallback still shows it (see setProviderModels below
            // for how this stays in sync with the live catalog once reachable).
            { value: "claude-fable-5", label: "Fable 5", description: "Claude Fable 5" },
        ],
    },
    codex: {
        id: "codex",
        displayName: "Codex CLI",
        cliCommand: "codex",
        defaultArgs: [],
        // exec subcommand runs non-interactively; --json emits NDJSON events; - reads prompt from stdin
        styledArgs: ["exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "-"],
        outputFormat: "raw",
        styledOutputFormat: "codex-json",
        authType: "oauth",
        authCheckCommand: ["login", "status"],
        authLoginCommand: ["login"],
        npmPackage: "@openai/codex",
        pinnedVersion: "0.116.0",
        docsUrl: "https://platform.openai.com/docs/codex",
        windowsInstallCommand: "npm install -g @openai/codex",
        unixInstallCommand: "npm install -g @openai/codex",
        icon: "robot",
        authConfigDirEnvVar: "CODEX_HOME",
        authDirName: "codex",
        supportedVendors: ["openai"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "-"],
        // Codex resume requires a subcommand change (exec resume <id>), not a simple flag.
        resumeFlag: null,
        resumeStrategy: "codex-exec",
        sessionIdField: "thread_id",
        controllerType: "subprocess",
        contextWindow: 200_000,
        // Verify ChatGPT-account availability when bumping the codex CLI pin.
        models: [
            { value: "gpt-5.5", label: "GPT-5.5", default: true, description: "Current codex frontier" },
            { value: "gpt-5.4", label: "GPT-5.4", description: "Prior frontier" },
            { value: "gpt-5.1-codex-max", label: "GPT-5.1-Codex-Max", description: "Codex-tuned, long-horizon" },
            { value: "gpt-5.3-codex", label: "GPT-5.3-Codex", description: "Codex-tuned" },
        ],
    },
    // muxcode — AgentMux's first-party agentic coding CLI.
    // Supports local GGUF inference via llama-server, Anthropic, OpenAI, and
    // OpenAI-compatible backends. Emits claude-stream-json NDJSON output.
    // npm: @agentmuxai/muxcode
    muxcode: {
        id: "muxcode",
        displayName: "Mux Code",
        cliCommand: "muxcode",
        defaultArgs: [],
        // muxcode emits NDJSON unconditionally — no --output-format flag.
        styledArgs: ["run", "-p"],
        outputFormat: "raw",
        styledOutputFormat: "claude-stream-json",
        authType: "api-key",
        // `auth status` exits 0 when any backend is ready (API key env var or
        // local GGUF model installed), exits 1 when nothing is configured.
        // Avoids the false-positive that `--version` would cause.
        authCheckCommand: ["auth", "status"],
        // `auth login` pulls a default local model when no backend is configured.
        authLoginCommand: ["auth", "login"],
        npmPackage: "@agentmuxai/muxcode",
        pinnedVersion: "0.1.0",
        docsUrl: "https://github.com/agentmuxai/muxcode",
        windowsInstallCommand: "npm install -g @agentmuxai/muxcode",
        unixInstallCommand: "npm install -g @agentmuxai/muxcode",
        icon: "brain",
        authConfigDirEnvVar: "MUXCODE_CONFIG_DIR",
        authDirName: "muxcode",
        supportedVendors: ["ollama", "anthropic", "openai"],
        startupInstructionsFilename: "CLAUDE.md",
        launchArgs: ["run", "-p"],
        resumeFlag: "--resume",
        sessionIdField: "session_id",
        controllerType: "subprocess",
        contextWindow: 200_000,
    },
    gemini: {
        id: "gemini",
        displayName: "Gemini CLI",
        cliCommand: "gemini",
        defaultArgs: [],
        // --output-format stream-json: NDJSON events; --yolo: auto-approve all tools;
        // -p "": enable headless/non-interactive mode (prompt comes from stdin)
        styledArgs: ["--output-format", "stream-json", "--yolo", "-p", ""],
        outputFormat: "raw",
        styledOutputFormat: "gemini-json",
        authType: "oauth",
        authCheckCommand: ["auth", "status"],
        authLoginCommand: ["auth", "login"],
        npmPackage: "@google/gemini-cli",
        pinnedVersion: "0.32.1",
        docsUrl: "https://ai.google.dev/gemini-cli",
        windowsInstallCommand: "npm install -g @google/gemini-cli",
        unixInstallCommand: "npm install -g @google/gemini-cli",
        icon: "diamond",
        authConfigDirEnvVar: "GEMINI_CLI_HOME",
        authDirName: "gemini",
        supportedVendors: ["google"],
        startupInstructionsFilename: "GEMINI.md",
        authExtraEnv: { GEMINI_FORCE_FILE_STORAGE: "true" },
        launchArgs: ["--output-format", "stream-json", "--yolo", "-p", ""],
        resumeFlag: "-r",
        sessionIdField: "session_id",
        controllerType: "subprocess",
        contextWindow: 1_000_000,
    },
    // Qwen Code — Alibaba's open-source coding agent, a fork of Gemini CLI.
    // Same stream-json headless surface → reuses the gemini translator
    // (styledOutputFormat "gemini-json"). Backend is OpenAI-compatible: set
    // OPENAI_BASE_URL=https://openrouter.ai/api/v1 + OPENAI_API_KEY (+ OPENAI_MODEL)
    // to run any OpenRouter model. The Qwen OAuth free tier was retired
    // (2026-04-15), so this is treated as api-key.
    // Auth: the intended path is an env-injected key from the identity bundle
    // (OPENAI_API_KEY/OPENROUTER_API_KEY), not an interactive CLI login. We use
    // the Gemini-parent `auth status`/`auth` convention for the check/login:
    // `auth status` fails-closed (prompts for auth) rather than reporting a
    // false positive the way `--version` would (checkcliauth treats any
    // non-JSON zero exit as authenticated — cli_handlers.rs). A deeper fix
    // would validate the bound env key directly in the api-key flow.
    qwen: {
        id: "qwen",
        displayName: "Qwen Code",
        cliCommand: "qwen",
        defaultArgs: [],
        styledArgs: ["--output-format", "stream-json", "--yolo", "-p", ""],
        outputFormat: "raw",
        styledOutputFormat: "gemini-json",
        authType: "api-key",
        authCheckCommand: ["auth", "status"],
        authLoginCommand: ["auth"],
        npmPackage: "@qwen-code/qwen-code",
        pinnedVersion: "0.19.2",
        docsUrl: "https://qwenlm.github.io/qwen-code-docs",
        windowsInstallCommand: "npm install -g @qwen-code/qwen-code",
        unixInstallCommand: "npm install -g @qwen-code/qwen-code",
        icon: "feather",
        authConfigDirEnvVar: "QWEN_HOME",
        authDirName: "qwen",
        supportedVendors: ["openrouter"],
        startupInstructionsFilename: "QWEN.md",
        launchArgs: ["--output-format", "stream-json", "--yolo", "-p", ""],
        resumeFlag: null,
        sessionIdField: "session_id",
        controllerType: "subprocess",
    },
    // OpenClaw — model-agnostic personal AI assistant from openclaw.ai.
    // We launch its `openclaw acp` bridge: speaks ACP over stdio (our
    // side) and forwards turns to OpenClaw's local Gateway daemon over
    // WebSocket (its side). The Gateway is OpenClaw's own daemon
    // (`openclaw gateway --port 18789`) and MUST be running before
    // the bridge can establish a session — onboarding via
    // `openclaw onboard` covers that on first install.
    //
    // OpenClaw is model-agnostic — the backing LLM brain is selected
    // by the user inside OpenClaw's own config (defaults to Pi; users
    // can wire Claude, Codex/OpenAI, Gemini, or local models via
    // `openclaw models auth login --provider <provider>`).
    openclaw: {
        id: "openclaw",
        displayName: "OpenClaw",
        cliCommand: "openclaw",
        defaultArgs: [],
        styledArgs: [],
        outputFormat: "acp",
        styledOutputFormat: "acp",
        // OAuth-via-subcommand. `openclaw models auth login --provider
        // openai-codex` runs OpenAI's "Sign in with ChatGPT" flow and
        // writes the resulting profile under ~/.openclaw/. OpenClaw then
        // uses that profile to spawn Codex's app-server as the agent's
        // backing brain (per docs.openclaw.ai/plugins/codex-harness +
        // SPEC_OPENCLAW_AGENT_2026_05_17.md §4).
        //
        // Future Phase: add more provider options ("Login with Claude",
        // "Login with Gemini", ...) and let the user pick which brain
        // before the OAuth subcommand runs.
        authType: "oauth",
        // `doctor` is a health/repair command — exits 0 even when no
        // openai-codex auth profile is registered, which would let
        // AgentMux skip the OAuth login and launch `openclaw acp`
        // unauthenticated. List the profiles for the specific provider
        // instead; exits non-zero when none are configured.
        authCheckCommand: ["models", "auth", "list", "--provider", "openai-codex"],
        // `cliCommand: "openclaw"` is prefixed by the spawn layer — keep
        // only the args here. Kimi's `["login"]` is the convention.
        authLoginCommand: ["models", "auth", "login", "--provider", "openai-codex"],
        npmPackage: "openclaw",
        pinnedVersion: "2026.6.10",
        docsUrl: "https://docs.openclaw.ai",
        windowsInstallCommand: "npm install -g openclaw",
        unixInstallCommand: "npm install -g openclaw",
        icon: "lobster",
        authConfigDirEnvVar: "OPENCLAW_HOME",
        authDirName: "openclaw",
        supportedVendors: ["openai", "anthropic", "google"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["acp"],
        resumeFlag: null,
        sessionIdField: "sessionId",
        controllerType: "acp",
        requiresLoginTty: true,
        // Same git dependency as Claude Code — OpenClaw uses git for
        // project-context features when invoking the Codex harness.
        systemPrereqs: [GIT_PREREQ],
        contextWindow: 200_000,
    },
    // Kimi Code CLI — Moonshot AI's coding agent.
    // Python-based CLI (not npm). Supports stream-json output and OpenAI-style tool calls.
    kimi: {
        id: "kimi",
        displayName: "Kimi Code CLI",
        cliCommand: "kimi",
        defaultArgs: [],
        styledArgs: ["--print", "--output-format", "stream-json", "--yolo", "-p", ""],
        outputFormat: "raw",
        styledOutputFormat: "kimi-stream-json",
        authType: "api-key",
        authCheckCommand: ["info"],
        authLoginCommand: ["login"],
        npmPackage: "",
        pinnedVersion: "",
        docsUrl: "https://moonshotai.github.io/kimi-cli/",
        windowsInstallCommand: "pip install kimi-cli",
        unixInstallCommand: "pip install kimi-cli",
        icon: "moon",
        unsetEnv: [],
        authConfigDirEnvVar: "KIMI_SHARE_DIR",
        authDirName: "kimi",
        supportedVendors: ["moonshot"],
        launchArgs: ["--print", "--output-format", "stream-json", "--yolo", "-p", ""],
        resumeFlag: null,
        sessionIdField: "session_id",
        controllerType: "subprocess",
        contextWindow: 128_000,
    },
    // GitHub Copilot CLI — Microsoft's coding agent.
    // Runs in ACP mode (`--acp` flag) so the existing ACP controller
    // can drive it the same way it drives Pi and OpenClaw. The CLI's
    // `-p`/`--prompt` non-interactive mode doesn't accept stdin for
    // the prompt yet (github/copilot-cli#96, #1046), so ACP is the
    // only path that composes with our existing subprocess+stdin
    // controller model. Documentation and CLI reference come from
    // discussion #493 (research-cli-context-files-2026-04-22).
    copilot: {
        id: "copilot",
        displayName: "GitHub Copilot CLI",
        cliCommand: "copilot",
        defaultArgs: [],
        styledArgs: ["--acp"],
        outputFormat: "acp",
        styledOutputFormat: "acp",
        authType: "oauth",
        authCheckCommand: ["auth", "status"],
        authLoginCommand: ["auth", "login"],
        npmPackage: "@github/copilot",
        pinnedVersion: "1.0.65",
        docsUrl: "https://docs.github.com/copilot/concepts/agents/about-copilot-cli",
        windowsInstallCommand: "npm install -g @github/copilot",
        unixInstallCommand: "npm install -g @github/copilot",
        icon: "github",
        authConfigDirEnvVar: "COPILOT_HOME",
        authDirName: "copilot",
        supportedVendors: ["github"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["--acp"],
        resumeFlag: null,
        sessionIdField: "sessionId",
        controllerType: "acp",
        contextWindow: 128_000,
    },
    // Pi — the lightweight coding agent that powers OpenClaw.
    // Standalone CLI, no gateway required. Pure coding agent with read/write/bash/edit tools.
    // Ideal when users want a fast, self-contained coding agent without the full OpenClaw stack.
    pi: {
        id: "pi",
        displayName: "Pi",
        cliCommand: "pi",
        defaultArgs: [],
        styledArgs: ["--json"],
        outputFormat: "acp",
        styledOutputFormat: "acp",
        authType: "api-key",
        authCheckCommand: ["config", "get", "provider"],
        authLoginCommand: ["config"],
        npmPackage: "@mariozechner/pi-coding-agent",
        pinnedVersion: "0.73.1",
        docsUrl: "https://github.com/badlogic/pi-mono",
        windowsInstallCommand: "npm install -g @mariozechner/pi-coding-agent",
        unixInstallCommand: "npm install -g @mariozechner/pi-coding-agent",
        icon: "terminal",
        authConfigDirEnvVar: "PI_HOME",
        authDirName: "pi",
        supportedVendors: ["pi"],
        startupInstructionsFilename: ".pi/APPEND_SYSTEM.md",
        launchArgs: ["--json"],
        resumeFlag: null,
        sessionIdField: "sessionId",
        controllerType: "acp",
    },
    // Antigravity (AGY) — Google's agentic coding CLI harness. Emits the
    // same stream-json NDJSON envelope as Gemini CLI (its sibling
    // harness), so it reuses the gemini translator (styledOutputFormat
    // "gemini-json"). Mirrors agentmux-srv/src/backend/providers.rs
    // `static ANTIGRAVITY`.
    antigravity: {
        id: "antigravity",
        displayName: "Antigravity (AGY)",
        cliCommand: "agy",
        defaultArgs: [],
        styledArgs: ["--output-format", "stream-json", "--yolo", "-p", ""],
        outputFormat: "raw",
        styledOutputFormat: "gemini-json",
        authType: "oauth",
        authCheckCommand: ["auth", "status"],
        authLoginCommand: ["auth", "login"],
        npmPackage: "@google/antigravity-cli",
        pinnedVersion: "1.0.0",
        docsUrl: "https://ai.google.dev/antigravity",
        windowsInstallCommand: "npm install -g @google/antigravity-cli",
        unixInstallCommand: "npm install -g @google/antigravity-cli",
        icon: "zap",
        authConfigDirEnvVar: "ANTIGRAVITY_CONFIG_DIR",
        authDirName: "antigravity",
        authExtraEnv: { ANTIGRAVITY_FORCE_FILE_STORAGE: "true" },
        supportedVendors: ["google"],
        startupInstructionsFilename: "GEMINI.md",
        launchArgs: ["--output-format", "stream-json", "--yolo", "-p", ""],
        resumeFlag: "-r",
        sessionIdField: "session_id",
        controllerType: "subprocess",
        contextWindow: 1_000_000,
        models: [
            { value: "gemini-3.6-flash", label: "Gemini 3.6 Flash", default: true, description: "Fast, highly capable frontier model with 1M context" },
            { value: "gemini-2.5-pro", label: "Gemini 2.5 Pro", description: "Deep reasoning and complex coding" },
            { value: "gemini-2.5-flash", label: "Gemini 2.5 Flash", description: "Balanced performance and speed" },
            { value: "gemini-2.0-flash-thinking", label: "Gemini 2.0 Flash Thinking", description: "Chain-of-thought agentic reasoning" },
        ],
    },
};

// Aliases for provider IDs from older databases or alternate naming
export const PROVIDER_ALIASES: Record<string, string> = {
    "claude-code": "claude",
    "claude_code": "claude",
    "codex-cli": "codex",
    "gemini-cli": "gemini",
    "qwen-code": "qwen",
    "qwen3-coder": "qwen",
    "kimi-cli": "kimi",
    "kimi_code": "kimi",
    "openclaw-cli": "openclaw",
    "open-claw": "openclaw",
    "copilot-cli": "copilot",
    "github-copilot": "copilot",
    "copilot_cli": "copilot",
    "mux-code": "muxcode",
    "mux_code": "muxcode",
    "agy": "antigravity",
    "antigravity-cli": "antigravity",
    "antigravity_cli": "antigravity",
};

export function resolveProviderAlias(id: string): string {
    return PROVIDER_ALIASES[id] ?? id;
}

/**
 * The vendor concept is computed, not stored: a non-empty
 * `modelVendorBaseUrl` means the harness has been redirected off its
 * default backend, so the effective vendor is "custom" regardless of what
 * `provider` declares as its default. An empty/absent override means the
 * harness is talking to its own default vendor — the first entry in that
 * provider's `supportedVendors` (falls back to the harness id itself for a
 * provider with no `supportedVendors` declared, e.g. a not-yet-cataloged
 * one from an older DB row).
 */
export function resolveEffectiveVendor(
    provider: string,
    modelVendorBaseUrl: string | undefined | null,
): string {
    if (modelVendorBaseUrl && modelVendorBaseUrl.trim().length > 0) {
        return "custom";
    }
    return PROVIDERS[provider]?.supportedVendors?.[0] ?? provider;
}
