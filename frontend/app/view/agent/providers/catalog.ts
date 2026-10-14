// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ProviderDefinition, SystemPrereq } from "./types";

/** Shared git prereq — claude-code calls `git` from session-start
 *  (issue anthropics/claude-code#29898), and openclaw uses git for
 *  project context. Curated install URLs per platform. */
const GIT_PREREQ: SystemPrereq = {
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

/** Shared Node.js prereq. Every provider below except kimi (pip-based) and
 *  antigravity (a native binary from Google's own installer) is
 *  installed via `npm install -g <npmPackage>` (AgentInstallModal ->
 *  install.start -> `Command::new("npm.cmd"/"npm")`, agentmux-srv's
 *  install_handlers.rs) — on a machine with no Node.js/npm, that spawn
 *  fails immediately with a raw, unfriendly OS error and no recovery path
 *  (confirmed live: fresh-PC onboarding investigation, tracking issue
 *  #2940). `git`/`node`/`npm`/`python` are already fully wired as
 *  one-click-installable ids in AgentPrereqModal's SYSTEM_INSTALLABLE_IDS
 *  and agentmux-srv's system_install_handlers.rs (winget/brew/pkexec) —
 *  this constant just routes providers through that already-working gate
 *  instead of letting them hit the raw npm-spawn failure. */
export const NODE_PREREQ: SystemPrereq = {
    tool: "node",
    label: "Node.js",
    installUrls: {
        windows: "https://nodejs.org/",
        macos: "https://nodejs.org/",
        linux: "https://nodejs.org/",
    },
    installLinkText: {
        windows: "Install Node.js",
        macos: "Install Node.js",
        linux: "Install Node.js",
    },
};

/** Separate from NODE_PREREQ deliberately (Codex P1, PR #2947): on Linux,
 *  the distro's `nodejs` package can be installed without `npm` (historically
 *  split packages on Debian/Ubuntu) — `where node`/`which node` succeeds
 *  while npm is genuinely missing. Probing only `node` would report this
 *  requirement satisfied and still send the user into the same unrecoverable
 *  npm-spawn crash NODE_PREREQ exists to prevent. */
const NPM_PREREQ: SystemPrereq = {
    tool: "npm",
    label: "npm",
    installUrls: {
        windows: "https://nodejs.org/",
        macos: "https://nodejs.org/",
        linux: "https://nodejs.org/",
    },
    installLinkText: {
        windows: "Install Node.js (includes npm)",
        macos: "Install Node.js (includes npm)",
        linux: "Install Node.js (includes npm)",
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
        // Keep in sync with crates/srv/src/backend/providers.rs `pinned_version`,
        // enforced by ./pin-consistency.test.ts.
        pinnedVersion: "2.1.288",
        docsUrl: "https://docs.anthropic.com/claude-code",
        windowsInstallCommand: "irm https://claude.ai/install.ps1 | iex",
        unixInstallCommand: "curl -fsSL https://claude.ai/install.sh | bash",
        icon: "wand-magic-sparkles",
        unsetEnv: ["CLAUDECODE"],
        authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
        authDirName: "claude",
        // Documented Claude Code behavior: redirects the CLI at a non-Anthropic
        // (or proxied) backend — Bedrock, Vertex, OpenRouter, a custom proxy.
        // Mirrors crates/srv/src/backend/providers.rs `base_url_env_var`.
        baseUrlEnvVar: "ANTHROPIC_BASE_URL",
        supportedVendors: ["anthropic"],
        startupInstructionsFilename: "CLAUDE.md",
        // `--exclude-dynamic-system-prompt-sections`: keeps the system prompt
        // byte-identical across agents so the prompt cache holds (#1964). Same
        // argv as `static CLAUDE` in agentmux-srv providers.rs, which
        // pin-consistency.test.ts now enforces.
        launchArgs: ["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--dangerously-skip-permissions", "--exclude-dynamic-system-prompt-sections"],
        resumeFlag: "--resume",
        sessionIdField: "session_id",
        // Persistent (bidirectional stream-json) + the Agent SDK CONTROL PROTOCOL
        // is the only way AskUserQuestion works headless: the CLI auto-rejects it
        // ("Error: Answer questions?") unless launched with `--permission-prompt-tool
        // stdio` and answered via a control_response. `--dangerously-skip-permissions`
        // DISABLES that routing, so it must NOT be here. The persistent controller's
        // control-frame handler (`handle_control_frame`) auto-allows ordinary tools to
        // preserve today's yolo UX and surfaces only AskUserQuestion to the user. Keep
        // in sync with `static CLAUDE` in crates/srv/src/backend/providers.rs.
        // controllerType selects persistentLaunchArgs over launchArgs in agent-model.ts.
        // Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
        controllerType: "persistent",
        persistentLaunchArgs: ["--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--exclude-dynamic-system-prompt-sections", "--permission-prompt-tool", "stdio", "--permission-mode", "default"],
        // Claude Code calls `git` at session-start (issue
        // anthropics/claude-code#29898). Without git the CLI fails
        // with `Error: Git is required but was not found.`.
        // Node 22+: `engines` of @anthropic-ai/claude-code. Older Node installs
        // fine and then fails at launch, so check it up front.
        systemPrereqs: [GIT_PREREQ, { ...NODE_PREREQ, minVersion: "22.0.0" }, NPM_PREREQ],
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
        // Descriptions are deliberately VERSION-FREE. The label carries the
        // version and is refreshed from the live catalog by `setProviderModels`;
        // a description that also named one could contradict it, which is
        // exactly what users saw — a row reading "Fable 5.1" above the text
        // "Claude Fable 5". Keep version numbers in `label` only.
        models: [
            { value: "opus", label: "Opus 5.5", description: "Highest quality", aliases: ["claude-opus"] },
            { value: "sonnet", label: "Sonnet 5.5", default: true, description: "Balanced speed and quality", aliases: ["claude-sonnet"] },
            { value: "haiku", label: "Haiku 4.5", description: "Fastest", aliases: ["claude-haiku"] },
            // No confirmed generic "fable" alias (unlike opus/sonnet/haiku above) —
            // `claude --help` documents `--model` as taking "an alias for the latest
            // model (e.g. 'sonnet' or 'opus') or a model's full name", and lists no
            // fable shorthand — so this pins the concrete model id directly. Same id
            // family already relied on by context-window.ts's 1M band. Unlike the
            // alias rows above, a concrete id does NOT self-resolve, so
            // `setProviderModels` refreshes this row's `value` as well as its label.
            { value: "claude-fable-5-1", label: "Fable 5.1", description: "Long-horizon work" },
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
        pinnedVersion: "0.154.0",
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
        systemPrereqs: [NODE_PREREQ, NPM_PREREQ],
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
        // The first version published to npm (0.1.0 never was). Mirrors
        // srv's providers.rs; every pinned version must be published first.
        pinnedVersion: "0.8.0",
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
        // The package's `engines`: Node 24.18.1 or later.
        systemPrereqs: [{ ...NODE_PREREQ, minVersion: "24.18.1" }, NPM_PREREQ],
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
        pinnedVersion: "0.62.0",
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
        // Node 20+: `engines` of @google/gemini-cli.
        systemPrereqs: [{ ...NODE_PREREQ, minVersion: "20.0.0" }, NPM_PREREQ],
    },
    // Qwen Code — Alibaba's open-source coding agent, a fork of Gemini CLI.
    // Its stream-json is NOT Gemini's: it emits Claude Code's frame shape
    // (system/assistant/user/result; verified on 0.24.0), read by the Claude
    // translator as "qwen-stream-json". Its usage counts cached tokens inside
    // input_tokens, so the context meter doesn't read it. Backend is OpenAI-compatible: set
    // OPENAI_BASE_URL=https://openrouter.ai/api/v1 + OPENAI_API_KEY (+ OPENAI_MODEL)
    // to run any OpenRouter model. The Qwen OAuth free tier was retired
    // (2026-04-15), so this is treated as api-key.
    // Auth: the intended path is an env-injected key from the agent's account
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
        styledArgs: ["--output-format", "stream-json", "--include-partial-messages", "--yolo", "-p", ""],
        outputFormat: "raw",
        styledOutputFormat: "qwen-stream-json",
        authType: "api-key",
        authCheckCommand: ["auth", "status"],
        authLoginCommand: ["auth"],
        npmPackage: "@qwen-code/qwen-code",
        pinnedVersion: "0.24.7",
        docsUrl: "https://qwenlm.github.io/qwen-code-docs",
        windowsInstallCommand: "npm install -g @qwen-code/qwen-code",
        unixInstallCommand: "npm install -g @qwen-code/qwen-code",
        icon: "feather",
        authConfigDirEnvVar: "QWEN_HOME",
        authDirName: "qwen",
        supportedVendors: ["openrouter"],
        startupInstructionsFilename: "QWEN.md",
        launchArgs: ["--output-format", "stream-json", "--include-partial-messages", "--yolo", "-p", ""],
        resumeFlag: null,
        sessionIdField: "session_id",
        controllerType: "subprocess",
        // Node 22+: `engines` of @qwen-code/qwen-code.
        systemPrereqs: [{ ...NODE_PREREQ, minVersion: "22.0.0" }, NPM_PREREQ],
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
        pinnedVersion: "2026.9.8",
        docsUrl: "https://docs.openclaw.ai",
        windowsInstallCommand: "npm install -g openclaw",
        unixInstallCommand: "npm install -g openclaw",
        icon: "shrimp",
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
        // Node 24.16+: OpenClaw's preinstall and the CLI itself refuse older
        // Node ("node:sqlite truncates TEXT at embedded NUL", nodejs/node#61954),
        // checked at 2026.9.4, 2026.9.7 and 2026.9.8.
        systemPrereqs: [GIT_PREREQ, { ...NODE_PREREQ, minVersion: "24.16.0" }, NPM_PREREQ],
    },
    // Kimi Code CLI — Moonshot AI's coding agent.
    // Python-based CLI (not npm). Supports stream-json output and OpenAI-style tool calls.
    kimi: {
        id: "kimi",
        displayName: "Kimi Code CLI",
        cliCommand: "kimi",
        defaultArgs: [],
        styledArgs: ["--print", "--output-format", "stream-json", "--yolo"],
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
        // No `-p ""`: kimi 1.37 rejects an empty prompt before reading stdin;
        // without -p, --print reads the prompt from stdin.
        launchArgs: ["--print", "--output-format", "stream-json", "--yolo"],
        resumeFlag: null,
        sessionIdField: "session_id",
        controllerType: "subprocess",
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
        pinnedVersion: "1.0.91",
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
        systemPrereqs: [NODE_PREREQ, NPM_PREREQ],
    },
    // Pi — the lightweight coding agent that powers OpenClaw.
    // Standalone CLI, no gateway required. Pure coding agent with read/write/bash/edit tools.
    // Ideal when users want a fast, self-contained coding agent without the full OpenClaw stack.
    pi: {
        id: "pi",
        displayName: "Pi",
        // pi speaks no ACP itself; the pi-acp adapter does, driving the pi
        // CLI installed beside it (`companionNpmPackages`). Mirrors
        // crates/srv/src/backend/providers.rs `static PI`;
        // SPEC_PI_HARNESS_VIA_PI_ACP_2026_10_06.md.
        cliCommand: "pi-acp",
        defaultArgs: [],
        styledArgs: [],
        outputFormat: "acp",
        styledOutputFormat: "acp",
        // pi keeps its logins and API keys in PI_CODING_AGENT_DIR (AgentMux's
        // shared Pi dir, `authConfigDirEnvVar`), one for all Pi agents, so
        // there's no account to bind. pi-acp has no status command; srv runs
        // the `pi` beside it as `pi --list-models`, which says "No models
        // available" until a provider is set up. Login is pi-acp's own
        // terminal method, which starts `pi` interactively.
        authType: "cli-managed",
        authCheckCommand: ["--list-models"],
        authLoginCommand: ["--terminal-login"],
        // A terminal-only login (pi-acp's documented terminal auth method):
        // URL capture would only burn its timeout before the terminal opens.
        headlessLoginUrlUnsupported: true,
        npmPackage: "pi-acp",
        pinnedVersion: "0.0.34",
        companionNpmPackages: [{ name: "@earendil-works/pi-coding-agent", version: "1.0.4" }],
        docsUrl: "https://github.com/badlogic/pi-mono",
        windowsInstallCommand: "npm install -g pi-acp @earendil-works/pi-coding-agent",
        unixInstallCommand: "npm install -g pi-acp @earendil-works/pi-coding-agent",
        icon: "terminal",
        authConfigDirEnvVar: "PI_CODING_AGENT_DIR",
        authDirName: "pi",
        supportedVendors: ["pi"],
        // AGENTS.md: pi skips the trust-protected .pi/APPEND_SYSTEM.md in
        // RPC mode (pi-acp). See `static PI` in providers.rs.
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: [],
        resumeFlag: null,
        sessionIdField: "sessionId",
        controllerType: "acp",
        // Node 22.19+: `engines` of @earendil-works/pi-coding-agent (pi-acp
        // needs 20+).
        systemPrereqs: [{ ...NODE_PREREQ, minVersion: "22.19.0" }, NPM_PREREQ],
    },
    // Antigravity (AGY) — Google's agentic coding CLI harness. Checked against
    // agy 1.1.11 (SPEC_ANTIGRAVITY_HARNESS_REAL_CLI_2026_10_06.md): its
    // stream-json is its own schema, so it has its own translator
    // (`agy-stream-json`); it ignores stdin and takes the prompt as `-p
    // <prompt>` (`promptArgFlag`); auto-approve is
    // `--dangerously-skip-permissions` and resume is `--conversation <id>`.
    // Mirrors crates/srv/src/backend/providers.rs `static ANTIGRAVITY`.
    antigravity: {
        id: "antigravity",
        displayName: "Antigravity (AGY)",
        cliCommand: "agy",
        defaultArgs: [],
        styledArgs: ["--output-format", "stream-json", "--dangerously-skip-permissions"],
        outputFormat: "raw",
        styledOutputFormat: "agy-stream-json",
        // agy signs in once per machine (system keyring, else Google
        // Sign-In) and reads no per-account config dir, so there is no
        // account to bind. `agy models` needs a signed-in account; signing
        // in is plain `agy`, which does it on first run. It has no `auth`
        // subcommand.
        authType: "cli-managed",
        authCheckCommand: ["models"],
        authLoginCommand: [],
        // Not on npm: `agy` is a native binary that Google's installer puts in
        // place and keeps up to date itself, so AgentMux can't install it.
        // Like kimi, launch finds it on PATH (or in the installer's folder,
        // `known_install_paths` in providers.rs) and otherwise shows the
        // command below.
        npmPackage: "",
        pinnedVersion: "",
        docsUrl: "https://antigravity.google/",
        windowsInstallCommand: "irm https://antigravity.google/cli/install.ps1 | iex",
        unixInstallCommand: "curl -fsSL https://antigravity.google/cli/install.sh | bash",
        icon: "zap",
        authConfigDirEnvVar: "ANTIGRAVITY_CONFIG_DIR",
        authDirName: "antigravity",
        supportedVendors: ["google"],
        startupInstructionsFilename: "GEMINI.md",
        launchArgs: ["--output-format", "stream-json", "--dangerously-skip-permissions"],
        promptArgFlag: "-p",
        resumeFlag: "--conversation",
        sessionIdField: "conversation_id",
        controllerType: "subprocess",
        // A native binary: no Node.js or npm needed.
        systemPrereqs: [],
        // From `agy models` (agy 1.1.11); the newest of each family.
        models: [
            { value: "gemini-3.8-flash-medium", label: "Gemini 3.8 Flash (Medium)", default: true, description: "Fast, balanced" },
            { value: "gemini-3.8-flash-high", label: "Gemini 3.8 Flash (High)", description: "Fast, more reasoning" },
            { value: "gemini-3.8-flash-low", label: "Gemini 3.8 Flash (Low)", description: "Fastest, least reasoning" },
            { value: "gemini-3.1-pro-high", label: "Gemini 3.1 Pro (High)", description: "Deepest reasoning" },
            { value: "gemini-3.1-pro-low", label: "Gemini 3.1 Pro (Low)", description: "Pro model, lighter reasoning" },
            { value: "claude-sonnet-4-6", label: "Claude Sonnet 4.6 (Thinking)", description: "Anthropic, via Antigravity" },
            { value: "claude-opus-4-6-thinking", label: "Claude Opus 4.6 (Thinking)", description: "Anthropic, via Antigravity" },
            { value: "gpt-oss-120b-medium", label: "GPT-OSS 120B (Medium)", description: "OpenAI's open-weight model" },
        ],
    },
    kilo: {
        id: "kilo",
        displayName: "Kilo Code CLI",
        // `kilo acp`: ACP over stdio. Mirrors crates/srv/src/backend/providers/kilo.rs.
        cliCommand: "kilo",
        defaultArgs: [],
        styledArgs: [],
        outputFormat: "acp",
        styledOutputFormat: "acp",
        // Runs signed out on Kilo Gateway's free models and keeps its own sign-in
        // (`kilo auth login`), so there is nothing to bind and no check to gate a
        // launch on: an empty check reads as "unknown" and the launch goes ahead.
        authType: "cli-managed",
        authCheckCommand: [],
        authLoginCommand: ["auth","login"],
        requiresLoginTty: true,
        headlessLoginUrlUnsupported: true,
        npmPackage: "@kilocode/cli",
        pinnedVersion: "7.8.8",
        docsUrl: "https://kilo.ai/docs/cli",
        windowsInstallCommand: "npm install -g @kilocode/cli",
        unixInstallCommand: "npm install -g @kilocode/cli",
        icon: "terminal",
        authConfigDirEnvVar: "XDG_DATA_HOME",
        authDirName: "kilo",
        authExtraEnv: { KILO_DISABLE_AUTOUPDATE: "1", KILO_DISABLE_CLAUDE_CODE_PROMPT: "1" },
        supportedVendors: ["kilo"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["acp"],
        resumeFlag: null,
        sessionIdField: "sessionId",
        controllerType: "acp",
        systemPrereqs: [NODE_PREREQ, NPM_PREREQ],
    },
    cline: {
        id: "cline",
        displayName: "Cline CLI",
        // `cline --acp`: ACP over stdio. Mirrors crates/srv/src/backend/providers/cline.rs.
        cliCommand: "cline",
        defaultArgs: [],
        styledArgs: [],
        outputFormat: "acp",
        styledOutputFormat: "acp",
        // Cline has no status command; signed out, its `session/new` fails with
        // "Authentication required", which the pane shows. Login is `cline auth`.
        authType: "cli-managed",
        authCheckCommand: [],
        authLoginCommand: ["auth"],
        requiresLoginTty: true,
        headlessLoginUrlUnsupported: true,
        npmPackage: "cline",
        pinnedVersion: "3.0.71",
        docsUrl: "https://docs.cline.bot/cli/cli-reference",
        windowsInstallCommand: "npm install -g cline",
        unixInstallCommand: "npm install -g cline",
        icon: "terminal",
        authConfigDirEnvVar: "CLINE_DIR",
        authDirName: "cline",
        authExtraEnv: { CLINE_NO_AUTO_UPDATE: "1" },
        supportedVendors: ["cline"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["--acp"],
        resumeFlag: null,
        sessionIdField: "sessionId",
        controllerType: "acp",
        systemPrereqs: [NODE_PREREQ, NPM_PREREQ],
    },
    opencode: {
        id: "opencode",
        displayName: "OpenCode",
        // `opencode acp`: ACP over stdio (the V1 line upstream's installer and docs
        // ship). Mirrors crates/srv/src/backend/providers/opencode.rs.
        cliCommand: "opencode",
        defaultArgs: [],
        styledArgs: [],
        // ACP read with the turn's token usage (translator-factory.ts).
        outputFormat: "opencode-acp",
        styledOutputFormat: "opencode-acp",
        // Runs signed out on OpenCode Zen's free models and keeps its own sign-in
        // (`opencode auth login`): nothing to bind, no check to gate a launch on.
        authType: "cli-managed",
        authCheckCommand: [],
        authLoginCommand: ["auth","login"],
        requiresLoginTty: true,
        headlessLoginUrlUnsupported: true,
        npmPackage: "opencode-ai",
        pinnedVersion: "1.19.0",
        docsUrl: "https://opencode.ai/docs/",
        windowsInstallCommand: "npm install -g opencode-ai",
        unixInstallCommand: "npm install -g opencode-ai",
        icon: "terminal",
        authConfigDirEnvVar: "XDG_DATA_HOME",
        authDirName: "opencode",
        authExtraEnv: { OPENCODE_DISABLE_AUTOUPDATE: "1", OPENCODE_DISABLE_CLAUDE_CODE_PROMPT: "1" },
        supportedVendors: ["opencode"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["acp"],
        resumeFlag: null,
        sessionIdField: "sessionId",
        controllerType: "acp",
        systemPrereqs: [NODE_PREREQ, NPM_PREREQ],
    },
    goose: {
        id: "goose",
        displayName: "goose",
        // `goose acp`: ACP over stdio. Not on npm: the user installs the release
        // (the install command); srv finds it. Mirrors crates/srv/src/backend/providers/goose.rs.
        cliCommand: "goose",
        defaultArgs: [],
        styledArgs: [],
        outputFormat: "acp",
        styledOutputFormat: "acp",
        // goose has no sign-in of its own: `goose configure` picks a model provider
        // and key. Until then its `session/new` fails naming GOOSE_PROVIDER, which
        // the pane shows. No status command, so no check.
        authType: "cli-managed",
        authCheckCommand: [],
        authLoginCommand: ["configure"],
        requiresLoginTty: true,
        headlessLoginUrlUnsupported: true,
        npmPackage: "",
        pinnedVersion: "1.54.0",
        versionSource: { kind: "github-release", repo: "aaif-goose/goose" },
        docsUrl: "https://goose-docs.ai/docs/getting-started/installation",
        windowsInstallCommand: "irm https://github.com/aaif-goose/goose/releases/download/stable/download_cli.ps1 | iex",
        unixInstallCommand: "curl -fsSL https://github.com/aaif-goose/goose/releases/download/stable/download_cli.sh | CONFIGURE=false bash",
        icon: "terminal",
        authConfigDirEnvVar: "GOOSE_PATH_ROOT",
        authDirName: "goose",
        authExtraEnv: { GOOSE_DISABLE_KEYRING: "1", GOOSE_TELEMETRY_OFF: "1" },
        supportedVendors: ["goose"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["acp"],
        resumeFlag: null,
        sessionIdField: "sessionId",
        controllerType: "acp",
    },
    grok: {
        id: "grok",
        displayName: "Grok Build",
        // `grok agent stdio`: ACP over stdio. Not on npm: the user installs it with
        // xAI's installer; srv finds it. Mirrors crates/srv/src/backend/providers/grok.rs.
        cliCommand: "grok",
        defaultArgs: [],
        styledArgs: [],
        outputFormat: "acp",
        styledOutputFormat: "acp",
        // Sign-in is Grok's own (`grok login`, a SuperGrok or X Premium+ account,
        // or XAI_API_KEY). Signed out, `session/new` fails "Authentication
        // required", which the pane shows. No status command, so no check.
        authType: "cli-managed",
        authCheckCommand: [],
        authLoginCommand: ["login"],
        requiresLoginTty: true,
        headlessLoginUrlUnsupported: true,
        npmPackage: "",
        pinnedVersion: "1.0.52",
        versionSource: { kind: "vendor", url: "https://x.ai/cli/stable" },
        docsUrl: "https://github.com/xai-org/grok-build",
        windowsInstallCommand: "irm https://x.ai/cli/install.ps1 | iex",
        unixInstallCommand: "curl -fsSL https://x.ai/cli/install.sh | bash",
        icon: "terminal",
        authConfigDirEnvVar: "GROK_HOME",
        authDirName: "grok",
        authExtraEnv: {
            GROK_DISABLE_AUTOUPDATER: "1",
            GROK_TELEMETRY_ENABLED: "false",
            GROK_CLAUDE_SKILLS_ENABLED: "0",
            GROK_CLAUDE_RULES_ENABLED: "0",
            GROK_CLAUDE_AGENTS_ENABLED: "0",
            GROK_CLAUDE_MCPS_ENABLED: "0",
            GROK_CLAUDE_HOOKS_ENABLED: "0",
            GROK_CLAUDE_SESSIONS_ENABLED: "0",
            GROK_CURSOR_SKILLS_ENABLED: "0",
            GROK_CURSOR_RULES_ENABLED: "0",
            GROK_CURSOR_AGENTS_ENABLED: "0",
            GROK_CURSOR_MCPS_ENABLED: "0",
            GROK_CURSOR_HOOKS_ENABLED: "0",
            GROK_CURSOR_SESSIONS_ENABLED: "0",
        },
        supportedVendors: ["xai"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["agent","stdio"],
        resumeFlag: null,
        sessionIdField: "sessionId",
        controllerType: "acp",
    },
    dsh: {
        id: "dsh",
        displayName: "DeepSeek Harness",
        // `dsh --profile acp`: ACP over stdio. A prerelease upstream, shown as a
        // preview. Mirrors crates/srv/src/backend/providers/dsh.rs.
        cliCommand: "dsh",
        defaultArgs: [],
        styledArgs: [],
        outputFormat: "acp",
        styledOutputFormat: "acp",
        // Runs signed out until a prompt needs DEEPSEEK_API_KEY. The key is set in
        // its web UI (`dsh web`, the Models page). No status command, so no check.
        authType: "cli-managed",
        authCheckCommand: [],
        authLoginCommand: ["web"],
        requiresLoginTty: true,
        headlessLoginUrlUnsupported: true,
        npmPackage: "@deepseek-ai/dsh",
        pinnedVersion: "0.2.0-rc.2",
        docsUrl: "https://github.com/deepseek-ai/deepseek-harness",
        windowsInstallCommand: "npm install -g @deepseek-ai/dsh",
        unixInstallCommand: "npm install -g @deepseek-ai/dsh",
        icon: "terminal",
        authConfigDirEnvVar: "DSH_HOME",
        authDirName: "dsh",
        authExtraEnv: { DSH_TELEMETRY_DISABLED: "1" },
        supportedVendors: ["deepseek"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["--profile","acp"],
        resumeFlag: null,
        sessionIdField: "sessionId",
        controllerType: "acp",
        systemPrereqs: [{ ...NODE_PREREQ, minVersion: "22.19.0" }, NPM_PREREQ],
    },
    aider: {
        id: "aider",
        displayName: "Aider",
        // Terminal tier: Aider's own UI in a terminal pane. Not on npm: the user
        // installs it with Aider's installer. Mirrors crates/srv/src/backend/providers/aider.rs.
        cliCommand: "aider",
        defaultArgs: [],
        styledArgs: [],
        outputFormat: "raw",
        styledOutputFormat: "raw",
        // No sign-in of its own: Aider takes the model provider's key from the
        // environment (OPENAI_API_KEY, ANTHROPIC_API_KEY, …). No check, no login.
        authType: "cli-managed",
        authCheckCommand: [],
        authLoginCommand: [],
        requiresLoginTty: true,
        headlessLoginUrlUnsupported: true,
        npmPackage: "",
        pinnedVersion: "0.86.2",
        versionSource: { kind: "pypi", package: "aider-chat" },
        docsUrl: "https://aider.chat/docs/install.html",
        windowsInstallCommand: "irm https://aider.chat/install.ps1 | iex",
        unixInstallCommand: "curl -LsSf https://aider.chat/install.sh | sh",
        icon: "terminal",
        authConfigDirEnvVar: "",
        authDirName: "aider",
        supportedVendors: ["aider"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["--read","AGENTS.md","--analytics-disable","--no-check-update","--no-show-release-notes","--no-gitignore"],
        resumeFlag: null,
        sessionIdField: "",
        controllerType: "terminal",
    },
    crush: {
        id: "crush",
        displayName: "Crush",
        // Terminal tier: Crush's own UI in a terminal pane (no ACP in this version).
        // Installed from Charm's own npm package, never bundled (its license).
        // Mirrors crates/srv/src/backend/providers/crush.rs.
        cliCommand: "crush",
        defaultArgs: [],
        styledArgs: [],
        outputFormat: "raw",
        styledOutputFormat: "raw",
        // Crush keeps its own sign-ins and keys (`crush login`, or set in its UI).
        authType: "cli-managed",
        authCheckCommand: [],
        authLoginCommand: ["login"],
        requiresLoginTty: true,
        headlessLoginUrlUnsupported: true,
        npmPackage: "@charmland/crush",
        pinnedVersion: "0.98.1",
        docsUrl: "https://github.com/charmbracelet/crush",
        windowsInstallCommand: "npm install -g @charmland/crush",
        unixInstallCommand: "npm install -g @charmland/crush",
        icon: "terminal",
        authConfigDirEnvVar: "CRUSH_GLOBAL_DATA",
        authDirName: "crush",
        authExtraEnv: { CRUSH_DISABLE_METRICS: "1", DO_NOT_TRACK: "1", CRUSH_DISABLE_PROVIDER_AUTO_UPDATE: "1" },
        supportedVendors: ["crush"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: [],
        resumeFlag: null,
        sessionIdField: "",
        controllerType: "terminal",
        systemPrereqs: [NODE_PREREQ, NPM_PREREQ],
    },
    muse: {
        id: "muse",
        displayName: "Muse Code",
        // Terminal tier for now: Muse Code's own UI in a terminal pane (it has no
        // ACP). Not on npm: the user installs it with Meta's installer; srv runs
        // its native binary. Mirrors crates/srv/src/backend/providers/muse.rs.
        cliCommand: "muse",
        defaultArgs: [],
        styledArgs: [],
        outputFormat: "raw",
        styledOutputFormat: "raw",
        // Sign-in is Muse's own (`muse login`, a Meta account, or META_API_KEY).
        authType: "cli-managed",
        authCheckCommand: [],
        authLoginCommand: ["login"],
        requiresLoginTty: true,
        headlessLoginUrlUnsupported: true,
        npmPackage: "",
        pinnedVersion: "1.4.7-R6101.1",
        versionSource: { kind: "vendor", url: "https://api.meta.ai/muse-code/channels/muse-stable" },
        docsUrl: "https://dev.meta.ai/docs/muse-code",
        windowsInstallCommand: "irm https://dev.meta.ai/install.ps1 | iex",
        unixInstallCommand: "curl -fsSL https://dev.meta.ai/install.sh | sh",
        icon: "terminal",
        authConfigDirEnvVar: "XDG_CONFIG_HOME",
        authDirName: "muse",
        authExtraEnv: { MUSE_NO_AUTO_UPDATE: "1", TBH_DISABLE_TELEMETRY: "1" },
        supportedVendors: ["meta"],
        startupInstructionsFilename: "AGENTS.md",
        launchArgs: ["--trust-workspace"],
        resumeFlag: null,
        sessionIdField: "",
        controllerType: "terminal",
    },
};

// Aliases for provider IDs from older databases or alternate naming
const PROVIDER_ALIASES: Record<string, string> = {
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
    "kilocode": "kilo",
    "kilo-code": "kilo",
    "cline-cli": "cline",
    "opencode-ai": "opencode",
    "open-code": "opencode",
    "goose-cli": "goose",
    "grok-build": "grok",
    "grok-cli": "grok",
    "deepseek-harness": "dsh",
    "aider-chat": "aider",
    "crush-cli": "crush",
    "muse-code": "muse",
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
