// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export interface ProviderDefinition {
    id: string;
    displayName: string;
    cliCommand: string;
    defaultArgs: string[];
    styledArgs: string[];        // CLI flags for JSON streaming mode
    outputFormat: "claude-stream-json" | "gemini-json" | "codex-json" | "raw";
    styledOutputFormat: "claude-stream-json" | "gemini-json" | "codex-json";
    authType: "oauth" | "api-key";
    authCheckCommand: string[];  // e.g. ["auth", "status", "--json"]
    authLoginCommand: string[];  // e.g. ["auth", "login"]
    npmPackage: string;          // npm package name for local install
    pinnedVersion: string;       // version to install ("latest" or specific)
    docsUrl: string;
    installCommand?: string;
    icon: string;
    unsetEnv?: string[];         // env vars to unset before launching (e.g. nested-session guards)
}

export const PROVIDERS: Record<string, ProviderDefinition> = {
    claude: {
        id: "claude",
        displayName: "Claude Code",
        cliCommand: "claude",
        defaultArgs: [],
        styledArgs: ["--output-format", "stream-json", "--verbose"],
        outputFormat: "raw",
        styledOutputFormat: "claude-stream-json",
        authType: "oauth",
        authCheckCommand: ["auth", "status", "--json"],
        authLoginCommand: ["auth", "login"],
        npmPackage: "@anthropic-ai/claude-code",
        pinnedVersion: "latest",
        docsUrl: "https://docs.anthropic.com/claude-code",
        icon: "sparkles",
        unsetEnv: ["CLAUDECODE"],
    },
    codex: {
        id: "codex",
        displayName: "Codex CLI",
        cliCommand: "codex",
        defaultArgs: [],
        styledArgs: ["--full-auto"],
        outputFormat: "raw",
        styledOutputFormat: "codex-json",
        authType: "oauth",
        authCheckCommand: ["login", "status"],
        authLoginCommand: ["login"],
        npmPackage: "@openai/codex",
        pinnedVersion: "0.107.0",
        docsUrl: "https://platform.openai.com/docs/codex",
        icon: "robot",
    },
    gemini: {
        id: "gemini",
        displayName: "Gemini CLI",
        cliCommand: "gemini",
        defaultArgs: [],
        styledArgs: ["--yolo"],
        outputFormat: "raw",
        styledOutputFormat: "gemini-json",
        authType: "oauth",
        authCheckCommand: ["auth", "status"],
        authLoginCommand: ["auth", "login"],
        npmPackage: "@google/gemini-cli",
        pinnedVersion: "0.31.0",
        docsUrl: "https://ai.google.dev/gemini-cli",
        icon: "diamond",
    },
};

export function getProvider(id: string): ProviderDefinition | undefined {
    return PROVIDERS[id];
}

export function getProviderList(): ProviderDefinition[] {
    return Object.values(PROVIDERS);
}
