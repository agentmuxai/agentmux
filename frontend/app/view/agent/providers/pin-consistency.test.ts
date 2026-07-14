// Drift guard for CLI version pins duplicated across registries.
//
// The pinned CLI version for each npm-installed provider lives in FOUR
// places that must agree (the follow-up SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG
// §"Single-source-of-truth" recommended and this test implements):
//
//   1. frontend/app/view/agent/providers/index.ts   `pinnedVersion`
//   2. agentmux-srv/src/backend/providers.rs        `pinned_version`
//   3. agentmux-cef/src/commands/providers.rs       `CLAUDE_VERSION` etc.
//   4. .github/workflows/container-image.yml        `claude_version` default
//      (claude only — the container image is a Claude agent image)
//
// History: the 2026-07-02 pin bump (2.1.185 → 2.1.198) updated #1 and #2 but
// missed #3 and #4, leaving the host-side installer 13 patch versions behind
// the srv-side installer for the same provider. This test makes the next
// missed site a CI failure instead of a silent drift.
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PROVIDERS } from "./index";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../..");

function read(rel: string): string {
    return readFileSync(resolve(repoRoot, rel), "utf8");
}

/** Extract `pinned_version: "X"` from a named `static NAME: ProviderConfig` block. */
function srvPin(source: string, staticName: string): string {
    const m = source.match(
        new RegExp(`static ${staticName}: ProviderConfig = ProviderConfig \\{[\\s\\S]*?pinned_version: "([^"]*)"`)
    );
    if (!m) throw new Error(`pinned_version not found for static ${staticName} in agentmux-srv providers.rs`);
    return m[1];
}

/** Extract `const NAME: &str = "X";` from the cef host installer. */
function cefPin(source: string, constName: string): string {
    const m = source.match(new RegExp(`const ${constName}: &str = "([^"]+)";`));
    if (!m) throw new Error(`const ${constName} not found in agentmux-cef providers.rs`);
    return m[1];
}

describe("CLI pin consistency across registries", () => {
    const srvSource = read("agentmux-srv/src/backend/providers.rs");
    const cefSource = read("agentmux-cef/src/commands/providers.rs");

    // provider key in PROVIDERS → [srv static name, cef const name]
    const registries: Array<[keyof typeof PROVIDERS & string, string, string]> = [
        ["claude", "CLAUDE", "CLAUDE_VERSION"],
        ["codex", "CODEX", "CODEX_VERSION"],
        ["gemini", "GEMINI", "GEMINI_VERSION"],
    ];

    for (const [key, srvStatic, cefConst] of registries) {
        it(`${key}: frontend, srv, and cef installer pins agree`, () => {
            const tsPin = PROVIDERS[key]?.pinnedVersion;
            expect(tsPin, `PROVIDERS.${key}.pinnedVersion missing`).toBeTruthy();
            expect(srvPin(srvSource, srvStatic), `srv pin for ${key}`).toBe(tsPin);
            expect(cefPin(cefSource, cefConst), `cef host pin for ${key}`).toBe(tsPin);
        });
    }

    it("claude: container-image.yml workflow default agrees", () => {
        const yml = read(".github/workflows/container-image.yml");
        const m = yml.match(/claude_version:[\s\S]*?default: '([^']+)'/);
        if (!m) throw new Error("claude_version default not found in container-image.yml");
        expect(m[1]).toBe(PROVIDERS.claude.pinnedVersion);
    });

    it("pins are concrete versions, not 'latest' (repeatable-install invariant)", () => {
        for (const [key] of registries) {
            expect(PROVIDERS[key].pinnedVersion, `${key} must be a concrete semver pin`).toMatch(/^\d+\.\d+\.\d+$/);
        }
    });
});
