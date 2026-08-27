// Drift guard for CLI version pins duplicated across registries.
//
// The pinned CLI version for each npm-installed provider lives in FIVE
// places that must agree (the follow-up SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG
// §"Single-source-of-truth" recommended and this test implements):
//
//   1. frontend/app/view/agent/providers/catalog.ts `pinnedVersion`
//      (re-exported as PROVIDERS via ./index — the module was a single
//      index.ts at pin #4 below's time; split for readability 2026-07-xx,
//      the pin moved but nothing re-audited references to the old path)
//   2. agentmux-srv/src/backend/providers.rs        `pinned_version`
//   3. agentmux-cef/src/commands/providers.rs       `CLAUDE_VERSION` etc.
//   4. .github/workflows/container-image.yml        `claude_version` default
//      (claude only — the container image is a Claude agent image)
//   5. docker/Dockerfile.agent-agentmux              `ARG CLAUDE_VERSION=`
//      (claude only, same reason as #4 — added 2026-08-27, see history below)
//
// History: the 2026-07-02 pin bump (2.1.185 → 2.1.198) updated #1 and #2 but
// missed #3 and #4, leaving the host-side installer 13 patch versions behind
// the srv-side installer for the same provider. This test made the next
// missed site a CI failure instead of a silent drift — but #5 (the Dockerfile
// ARG) wasn't covered at all until the 2026-08-27 bump (2.1.198 → 2.1.247)
// found it via `docs/spec-claude-code-versioning.md`'s own written checklist,
// which already warned this file could drift silently. Added here so a
// missed Dockerfile pin is now caught the same way #3/#4 are. See
// docs/retro/retro-claude-cli-and-opus-5-upgrade-2026-08-27.md and
// docs/specs/SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md for the fuller
// story of why a written checklist alone wasn't enough here.
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

    // Added 2026-08-27 — this location shipped a real, undetected drift risk
    // (docs/spec-claude-code-versioning.md's own hand-maintained checklist
    // had warned about it since the doc was first written, but nothing
    // machine-checked it until now). See this file's header comment.
    it("claude: Dockerfile.agent-agentmux ARG default agrees", () => {
        const dockerfile = read("docker/Dockerfile.agent-agentmux");
        const m = dockerfile.match(/ARG CLAUDE_VERSION=([^\s\n]+)/);
        if (!m) throw new Error("ARG CLAUDE_VERSION not found in docker/Dockerfile.agent-agentmux");
        expect(m[1]).toBe(PROVIDERS.claude.pinnedVersion);
    });

    it("pins are concrete versions, not 'latest' (repeatable-install invariant)", () => {
        for (const [key] of registries) {
            expect(PROVIDERS[key].pinnedVersion, `${key} must be a concrete semver pin`).toMatch(/^\d+\.\d+\.\d+$/);
        }
    });
});
