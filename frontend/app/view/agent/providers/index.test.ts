// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect } from "vitest";
import { PROVIDERS, getProvider, getProviderList } from "./index";
import type { ProviderDefinition } from "./index";

describe("PROVIDERS", () => {
    test("defines exactly 3 providers", () => {
        const ids = Object.keys(PROVIDERS);
        expect(ids).toHaveLength(3);
        expect(ids).toContain("claude");
        expect(ids).toContain("codex");
        expect(ids).toContain("gemini");
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
            expect(provider.npmPackage).toBeTruthy();
            expect(provider.pinnedVersion).toBeTruthy();
            expect(provider.docsUrl).toBeTruthy();
            expect(provider.icon).toBeTruthy();
        }
    });

    test("all providers are in raw output mode", () => {
        for (const provider of Object.values(PROVIDERS)) {
            expect(provider.outputFormat).toBe("raw");
            expect(provider.defaultArgs).toEqual([]);
        }
    });

    test("all providers use OAuth auth type", () => {
        for (const provider of Object.values(PROVIDERS)) {
            expect(provider.authType).toBe("oauth");
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
        expect(gemini.npmPackage).toBe("@anthropic-ai/gemini-cli");
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
        expect(list).toHaveLength(3);
        expect(list.map((p) => p.id)).toEqual(expect.arrayContaining(["claude", "codex", "gemini"]));
    });
});
