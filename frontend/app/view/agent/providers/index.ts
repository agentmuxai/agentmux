// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export interface ProviderDefinition {
    id: string;
    displayName: string;
    cliCommand: string;
    defaultArgs: string[];
    outputFormat: "claude-stream-json" | "gemini-json" | "codex-json" | "raw";
    authType: "oauth" | "api-key";
    authCheckCommand: string[];  // e.g. ["auth", "status", "--json"]
    authLoginCommand: string[];  // e.g. ["auth", "login"]
    npmPackage: string;          // npm package name for local install
    pinnedVersion: string;       // version to install ("latest" or specific)
    docsUrl: string;
    icon: string;
}

export const PROVIDERS: Record<string, ProviderDefinition> = {
    claude: {
        id: "claude",
        displayName: "Claude Code",
        cliCommand: "claude",
        defaultArgs: [],  // raw mode — no stream-json for now
        outputFormat: "raw",
        authType: "oauth",
        authCheckCommand: ["auth", "status", "--json"],
        authLoginCommand: ["auth", "login"],
        npmPackage: "@anthropic-ai/claude-code",
        pinnedVersion: "latest",
        docsUrl: "https://docs.anthropic.com/claude-code",
        icon: "sparkles",
    },
    codex: {
        id: "codex",
        displayName: "Codex CLI",
        cliCommand: "codex",
        defaultArgs: [],  // raw mode
        outputFormat: "raw",
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
        defaultArgs: [],  // raw mode
        outputFormat: "raw",
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
