// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export interface ProviderDefinition {
    id: string;
    displayName: string;
    cliCommand: string;
    defaultArgs: string[];
    styledArgs: string[];        // CLI flags for JSON streaming mode (documentation; use launchArgs for actual invocation)
    outputFormat: "claude-stream-json" | "gemini-json" | "codex-json" | "raw";
    styledOutputFormat: "claude-stream-json" | "gemini-json" | "codex-json";
    authType: "oauth" | "api-key";
    authCheckCommand: string[];  // e.g. ["auth", "status", "--json"]
    authLoginCommand: string[];  // e.g. ["auth", "login"]
    npmPackage: string;          // npm package name for local install
    pinnedVersion: string;       // version to install ("latest" or specific)
    docsUrl: string;
    windowsInstallCommand: string;  // official installer for Windows (powershell)
    unixInstallCommand: string;      // official installer for macOS/Linux (bash)
    icon: string;
    unsetEnv?: string[];         // env vars to unset before launching (e.g. nested-session guards)
    // Auth isolation — each provider gets its own versioned config dir
    authConfigDirEnvVar: string;        // env var that redirects the provider's config/auth dir
    authDirName: string;                // subdir name under {dataDir}/auth/ (e.g. "claude")
    authExtraEnv?: Record<string, string>;  // extra env vars needed for auth isolation (e.g. GEMINI_FORCE_FILE_STORAGE)
    // Launch args — the complete CLI args for a single turn (replaces hardcoded ["-p", ...styledArgs])
    // The user message is written to subprocess stdin; these args put the CLI in non-interactive mode.
    launchArgs: string[];
    // Resume flag — how to pass a session ID for multi-turn continuity.
    // null means this provider does not support simple-flag resume (e.g. Codex uses a subcommand).
    resumeFlag: string | null;
    // JSON field name containing the session/thread ID in the CLI's init event.
    sessionIdField: string;
}

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
        authLoginCommand: ["auth", "login"],
        npmPackage: "@anthropic-ai/claude-code",
        pinnedVersion: "latest",
        docsUrl: "https://docs.anthropic.com/claude-code",
        windowsInstallCommand: "irm https://claude.ai/install.ps1 | iex",
        unixInstallCommand: "curl -fsSL https://claude.ai/install.sh | bash",
        icon: "sparkles",
        unsetEnv: ["CLAUDECODE"],
        authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
        authDirName: "claude",
        launchArgs: ["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--dangerously-skip-permissions"],
        resumeFlag: "--resume",
        sessionIdField: "session_id",
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
        launchArgs: ["exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "-"],
        // Codex resume requires a subcommand change (exec resume <id>), not a simple flag.
        // Multi-turn is handled by re-running exec; null disables automatic --resume append.
        resumeFlag: null,
        sessionIdField: "thread_id",
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
        authExtraEnv: { GEMINI_FORCE_FILE_STORAGE: "true" },
        launchArgs: ["--output-format", "stream-json", "--yolo", "-p", ""],
        resumeFlag: "-r",
        sessionIdField: "session_id",
    },
};

// Aliases for provider IDs from older databases or alternate naming
const PROVIDER_ALIASES: Record<string, string> = {
    "claude-code": "claude",
    "claude_code": "claude",
    "codex-cli": "codex",
    "gemini-cli": "gemini",
};

export function resolveProviderAlias(id: string): string {
    return PROVIDER_ALIASES[id] ?? id;
}

export function getProvider(id: string): ProviderDefinition | undefined {
    return PROVIDERS[id] ?? PROVIDERS[resolveProviderAlias(id)];
}

export function getProviderList(): ProviderDefinition[] {
    return Object.values(PROVIDERS);
}
