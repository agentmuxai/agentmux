// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect } from "vitest";
import { PROVIDERS, getProvider, getProviderList } from "./index";
import type { ProviderDefinition } from "./index";

// Providers split into two classes by how they talk to the backend:
//   - OAuth CLIs (raw stream): claude, codex, gemini — authType "oauth",
//     outputFormat "raw", no defaultArgs.
//   - ACP providers:           openclaw, pi — authType "api-key",
//     outputFormat "acp".
const OAUTH_CLI_IDS = ["claude", "codex", "gemini"] as const;
const API_KEY_CLI_IDS = ["kimi"] as const;
const ACP_IDS = ["openclaw", "pi"] as const;

describe("PROVIDERS", () => {
    test("includes the OAuth CLI trio and the ACP providers", () => {
        const ids = Object.keys(PROVIDERS);
        for (const id of OAUTH_CLI_IDS) expect(ids).toContain(id);
        for (const id of ACP_IDS) expect(ids).toContain(id);
    });

    test("all providers have required fields", () => {
        for (const [id, provider] of Object.entries(PROVIDERS)) {
            expect(provider.id).toBe(id);
            expect(provider.displayName).toBeTruthy();
            expect(provider.cliCommand).toBeTruthy();
            expect(Array.isArray(provider.defaultArgs)).toBe(true);
            expect(provider.outputFormat).toBeTruthy();
            expect(provider.authType).toBeTruthy();
            expect(Array.isArray(provider.authCheckCommand)).toBe(true);
            expect(Array.isArray(provider.authLoginCommand)).toBe(true);
            expect(provider.docsUrl).toBeTruthy();
            expect(provider.icon).toBeTruthy();
        }
    });

    test("OAuth CLI providers are in raw output mode with no default args", () => {
        for (const id of OAUTH_CLI_IDS) {
            const provider = PROVIDERS[id];
            expect(provider.outputFormat).toBe("raw");
            expect(provider.defaultArgs).toEqual([]);
        }
    });

    test("API-key CLI providers are in raw output mode with no default args", () => {
        for (const id of API_KEY_CLI_IDS) {
            const provider = PROVIDERS[id];
            expect(provider.outputFormat).toBe("raw");
            expect(provider.defaultArgs).toEqual([]);
        }
    });

    test("OAuth CLI providers use OAuth auth type", () => {
        for (const id of OAUTH_CLI_IDS) {
            expect(PROVIDERS[id].authType).toBe("oauth");
        }
    });

    test("ACP providers use the ACP output format and api-key auth", () => {
        for (const id of ACP_IDS) {
            const provider = PROVIDERS[id];
            expect(provider.outputFormat).toBe("acp");
            expect(provider.authType).toBe("api-key");
        }
    });
});

describe("claude provider", () => {
    const claude = PROVIDERS.claude;

    test("has correct CLI command", () => {
        expect(claude.cliCommand).toBe("claude");
    });

    test("has correct auth commands", () => {
        expect(claude.authCheckCommand).toEqual(["auth", "status", "--json"]);
        expect(claude.authLoginCommand).toEqual(["auth", "login"]);
    });

    test("has correct npm package", () => {
        expect(claude.npmPackage).toBe("@anthropic-ai/claude-code");
    });
});

describe("codex provider", () => {
    const codex = PROVIDERS.codex;

    test("has correct CLI command", () => {
        expect(codex.cliCommand).toBe("codex");
    });

    test("has correct auth commands", () => {
        expect(codex.authCheckCommand).toEqual(["login", "status"]);
        expect(codex.authLoginCommand).toEqual(["login"]);
    });

    test("has correct npm package", () => {
        expect(codex.npmPackage).toBe("@openai/codex");
    });
});

describe("kimi provider", () => {
    const kimi = PROVIDERS.kimi;

    test("has correct CLI command", () => {
        expect(kimi.cliCommand).toBe("kimi");
    });

    test("has correct auth commands", () => {
        expect(kimi.authCheckCommand).toEqual(["info"]);
        expect(kimi.authLoginCommand).toEqual(["login"]);
    });

    test("has empty npm package (python-based CLI)", () => {
        expect(kimi.npmPackage).toBe("");
        expect(kimi.pinnedVersion).toBe("");
    });

    test("uses subprocess controller", () => {
        expect(kimi.controllerType).toBe("subprocess");
    });

    test("has kimi-stream-json styled output format", () => {
        expect(kimi.styledOutputFormat).toBe("kimi-stream-json");
    });
});

describe("gemini provider", () => {
    const gemini = PROVIDERS.gemini;

    test("has correct CLI command", () => {
        expect(gemini.cliCommand).toBe("gemini");
    });

    test("has correct auth commands", () => {
        expect(gemini.authCheckCommand).toEqual(["auth", "status"]);
        expect(gemini.authLoginCommand).toEqual(["auth", "login"]);
    });

    test("has correct npm package", () => {
        expect(gemini.npmPackage).toBe("@google/gemini-cli");
    });
});

describe("getProvider", () => {
    test("returns provider by id", () => {
        const claude = getProvider("claude");
        expect(claude).toBeDefined();
        expect(claude!.id).toBe("claude");
    });

    test("returns undefined for unknown id", () => {
        const unknown = getProvider("unknown");
        expect(unknown).toBeUndefined();
    });
});

describe("getProviderList", () => {
    test("returns all providers as array", () => {
        const list = getProviderList();
        expect(list).toHaveLength(Object.keys(PROVIDERS).length);
        expect(list.map((p) => p.id)).toEqual(
            expect.arrayContaining([...OAUTH_CLI_IDS, ...ACP_IDS]),
        );
    });
});
