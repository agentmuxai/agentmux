// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect } from "vitest";
import { PROVIDERS, getProvider, getProviderList } from "./index";
import type { ProviderDefinition } from "./index";

// Providers split into two classes by how they talk to the backend:
//   - OAuth CLIs (raw stream): claude, codex, gemini — authType "oauth",
//     outputFormat "raw", no defaultArgs.
//   - ACP providers:           openclaw (OAuth via subcommand —
//     SPEC_OPENCLAW_AGENT_2026_05_17.md §4), pi (api-key). Both
//     `outputFormat: "acp"`.
const OAUTH_CLI_IDS = ["claude", "codex", "gemini"] as const;
const API_KEY_CLI_IDS = ["kimi", "qwen"] as const;
const ACP_IDS = ["openclaw", "pi"] as const;
// ACP sub-partition by auth type. Kept separate from the unified
// `ACP_IDS` so the "ACP providers use the ACP output format"
// assertion can stay in one loop while the auth assertions live
// in their own loops below.
const ACP_OAUTH_IDS = ["openclaw"] as const;
const ACP_API_KEY_IDS = ["pi"] as const;

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

    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2's
    // per-provider table, pinned so a future edit can't silently drift from
    // the researched/cited values. Mirrors the Rust registry's
    // `startup_instructions_filename_matches_researched_table` test.
    test("startupInstructionsFilename matches researched table", () => {
        const expected: Record<string, string | undefined> = {
            claude: "CLAUDE.md",
            codex: "AGENTS.md",
            gemini: "GEMINI.md",
            qwen: "QWEN.md",
            copilot: "AGENTS.md",
            openclaw: "AGENTS.md",
            pi: ".pi/APPEND_SYSTEM.md",
            antigravity: "GEMINI.md",
            muxcode: "CLAUDE.md",
            kimi: undefined,
        };
        for (const [id, filename] of Object.entries(expected)) {
            expect(PROVIDERS[id]?.startupInstructionsFilename).toBe(filename);
        }
    });

    test("kimi has no startupInstructionsFilename — no confirmed native startup-instructions file", () => {
        expect(PROVIDERS.kimi.startupInstructionsFilename).toBeUndefined();
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

    test("ACP providers use the ACP output format", () => {
        // The output-format invariant is uniform across ACP
        // providers — they all speak ACP regardless of how they
        // authenticate.
        for (const id of ACP_IDS) {
            expect(PROVIDERS[id].outputFormat).toBe("acp");
        }
    });

    test("ACP+OAuth providers use OAuth auth (e.g. openclaw — OAuth via subcommand)", () => {
        // SPEC_OPENCLAW_AGENT_2026_05_17.md §4: openclaw runs an
        // OAuth flow via `models auth login --provider …` to
        // borrow another CLI's credentials (currently OpenAI
        // Codex), so its `authType` is "oauth" despite being an
        // ACP provider.
        for (const id of ACP_OAUTH_IDS) {
            expect(PROVIDERS[id].authType).toBe("oauth");
        }
    });

    test("ACP+api-key providers use api-key auth (e.g. pi)", () => {
        for (const id of ACP_API_KEY_IDS) {
            expect(PROVIDERS[id].authType).toBe("api-key");
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
