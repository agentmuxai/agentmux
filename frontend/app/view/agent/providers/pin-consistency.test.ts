// Drift guard for CLI version pins duplicated across registries.
//
// The pinned CLI version for each npm-installed provider lives in TWO
// places that must agree (the follow-up SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG
// §"Single-source-of-truth" recommended and this test implements). Until
// 2026-10-07 the container image build carried two more copies (a workflow
// input and a Dockerfile ARG); the image no longer contains the CLI, so those
// are gone and the tests below check they stay gone:
//
//   1. frontend/app/view/agent/providers/catalog.ts `pinnedVersion`
//      (re-exported as PROVIDERS via ./index)
//   2. crates/srv/src/backend/providers.rs        `pinned_version`
//      (also what container agents install on first start)
//
// History: the 2026-07-02 pin bump (2.1.185 → 2.1.198) updated the frontend
// and srv pins but missed the (since removed) CEF host installer and the
// container workflow, leaving the host-side installer 13 patch versions behind
// the srv-side installer for the same provider. This test made the next
// missed site a CI failure instead of a silent drift — but the Dockerfile ARG
// wasn't covered at all until the 2026-08-27 bump (2.1.198 → 2.1.247)
// found it via `docs/spec-claude-code-versioning.md`'s own written checklist,
// which already warned this file could drift silently. Added here so a
// missed Dockerfile pin is now caught the same way the others are. See
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

/**
 * The string literals of `field: &[ … ]` or `field: Some(&[ … ])` in a named
 * `static NAME: ProviderConfig` block, comments stripped. `null` for
 * `field: None`.
 */
function srvArgs(source: string, staticName: string, field: string): string[] | null {
    const block = source.match(new RegExp(`static ${staticName}: ProviderConfig = ProviderConfig \\{[\\s\\S]*?\\n\\};`));
    if (!block) throw new Error(`static ${staticName} not found in agentmux-srv providers.rs`);
    // Top-level fields only (4-space indent): codex nests its own
    // `launch_args` inside `app_server: Some(AppServerConfig { … })`.
    const m = block[0].match(new RegExp(`\\n {4}${field}: (None|Some\\(&\\[([\\s\\S]*?)\\]\\)|&\\[([\\s\\S]*?)\\])`));
    if (!m) throw new Error(`${field} not found for static ${staticName} in agentmux-srv providers.rs`);
    if (m[1] === "None") return null;
    const body = (m[2] ?? m[3]).replace(/\/\/[^\n]*/g, "");
    return [...body.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((s) => s[1]);
}

describe("CLI pin consistency across registries", () => {
    const srvSource = read("crates/srv/src/backend/providers.rs");

    // provider key in PROVIDERS → srv static name
    const registries: Array<[keyof typeof PROVIDERS & string, string]> = [
        ["claude", "CLAUDE"],
        ["codex", "CODEX"],
        ["gemini", "GEMINI"],
    ];

    for (const [key, srvStatic] of registries) {
        it(`${key}: frontend and srv installer pins agree`, () => {
            const tsPin = PROVIDERS[key]?.pinnedVersion;
            expect(tsPin, `PROVIDERS.${key}.pinnedVersion missing`).toBeTruthy();
            expect(srvPin(srvSource, srvStatic), `srv pin for ${key}`).toBe(tsPin);
        });
    }

    // The container image is the public base image: it must not carry Claude
    // Code (proprietary license, redistribution not cleared), and so has no pin
    // of its own to drift. The CLI is installed on first start from the pin above.
    it("claude: the published container image does not bundle Claude Code or pin its version", () => {
        const dockerfile = read("docker/Dockerfile.agent-agentmux");
        const instructions = dockerfile
            .split("\n")
            .filter((line) => !line.trimStart().startsWith("#"))
            .join("\n");
        expect(instructions).not.toMatch(/claude-code/);
        expect(instructions).not.toMatch(/CLAUDE_VERSION/);

        const yml = read(".github/workflows/container-image.yml");
        expect(yml).not.toMatch(/claude_version/);
        expect(yml).not.toMatch(/CLAUDE_VERSION/);
        expect(yml).toMatch(/IMAGE_NAME: agentmuxai\/agent-base$/m);
    });

    it("claude: the image has the directory the first-start install writes to", () => {
        const dockerfile = read("docker/Dockerfile.agent-agentmux");
        expect(dockerfile).toMatch(/mkdir -p \/home\/agent\/\.agentmux\/cli/);
        expect(dockerfile).toMatch(/chown -R agent:agent \/home\/agent\/\.agentmux/);
    });

    // Launch argv is duplicated the same way: srv's `launch_args` /
    // `persistent_launch_args` are what `agent.open` writes into `cmd:args`,
    // and the catalog's `launchArgs` / `persistentLaunchArgs` are what a pane
    // launched from the UI writes. Nothing compared them, and claude's drifted:
    // #1964 added `--exclude-dynamic-system-prompt-sections` to srv only, so the
    // flag depended on how the pane was created
    // (docs/specs/SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §5.1 #4).
    // Every provider srv defines, found from the source rather than listed by
    // hand, so a new provider can't be missed (ReAgent P1 on #4029: an earlier
    // hand-written list skipped pi, muxcode, copilot and antigravity).
    const argRegistries: Array<[keyof typeof PROVIDERS & string, string]> = [
        ...srvSource.matchAll(/static ([A-Z_]+): ProviderConfig = ProviderConfig \{\n\s+id: "([^"]+)"/g),
    ].map((m) => [m[2] as keyof typeof PROVIDERS & string, m[1]]);

    it("both registries define the same providers", () => {
        expect(argRegistries.length, "no ProviderConfig statics found in providers.rs").toBeGreaterThan(0);
        expect(argRegistries.map(([key]) => key).sort()).toEqual(Object.keys(PROVIDERS).sort());
    });

    for (const [key, srvStatic] of argRegistries) {
        it(`${key}: frontend and srv launch args agree`, () => {
            expect(PROVIDERS[key].launchArgs ?? [], `launchArgs for ${key}`).toEqual(
                srvArgs(srvSource, srvStatic, "launch_args") ?? []
            );
        });

        it(`${key}: frontend and srv persistent launch args agree`, () => {
            expect(PROVIDERS[key].persistentLaunchArgs ?? null, `persistentLaunchArgs for ${key}`).toEqual(
                srvArgs(srvSource, srvStatic, "persistent_launch_args")
            );
        });
    }

    it("pins are concrete versions, not 'latest' (repeatable-install invariant)", () => {
        for (const [key] of registries) {
            expect(PROVIDERS[key].pinnedVersion, `${key} must be a concrete semver pin`).toMatch(/^\d+\.\d+\.\d+$/);
        }
    });
});
