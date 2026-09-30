// Drift guard for CLI version pins duplicated across registries.
//
// The pinned CLI version for each npm-installed provider lives in FOUR
// places that must agree (the follow-up SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG
// §"Single-source-of-truth" recommended and this test implements). A fifth,
// the CEF host's own installer (`agentmux-cef/src/commands/providers.rs`
// CLAUDE_VERSION etc.), was removed on 2026-09-26 along with that unused
// installer — srv's install.* commands are the only installer:
//
//   1. frontend/app/view/agent/providers/catalog.ts `pinnedVersion`
//      (re-exported as PROVIDERS via ./index — the module was a single
//      index.ts at pin #4 below's time; split for readability 2026-07-xx,
//      the pin moved but nothing re-audited references to the old path)
//   2. agentmux-srv/src/backend/providers.rs        `pinned_version`
//   3. .github/workflows/container-image.yml        `claude_version` default
//      (claude only — the container image is a Claude agent image)
//   4. docker/Dockerfile.agent-agentmux              `ARG CLAUDE_VERSION=`
//      (claude only, same reason as #3 — added 2026-08-27, see history below)
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
    const srvSource = read("agentmux-srv/src/backend/providers.rs");

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
