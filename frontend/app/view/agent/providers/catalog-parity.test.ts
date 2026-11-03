// Drift guard for every field the frontend catalog (catalog.ts) and srv's `ProviderConfig` both carry.
// A pane opens from the UI with the catalog's values, or from `agent.open`, a layout or a container with
// srv's, so a field that differs makes the pane depend on how it was made. Each test below names a catalog
// field and its srv counterpart. launchArgs and the default model stay in pin-consistency.test.ts and
// runtime-defaults-consistency.test.ts. The providers come from srv's source, so a new one is compared with
// no edit here.
import { describe, expect, it } from "vitest";
import { PROVIDERS } from "./index";
import {
    read,
    readSrvProviders,
    srvField,
    srvOptString,
    srvPairs,
    srvStatics,
    srvString,
    srvStringList,
} from "./srv-source";

const source = readSrvProviders();
const statics = srvStatics(source);
const providersRs = read("crates/srv/src/backend/providers.rs");

/** srv's `ControllerType` variant → the catalog's `controllerType`. */
const CONTROLLER: Record<string, string> = {
    Persistent: "persistent",
    Subprocess: "subprocess",
    Acp: "acp",
    Msp: "msp",
    Terminal: "terminal",
};

/** `companion_npm_packages`'s match arms: provider id → `{ name, version }[]`. */
function srvCompanions(): Map<string, { name: string; version: string }[]> {
    const fn = providersRs.match(/pub fn companion_npm_packages\(&self\)[^{]*\{([\s\S]*?)\n    \}/);
    if (!fn) throw new Error("companion_npm_packages not found in providers.rs");
    const out = new Map<string, { name: string; version: string }[]>();
    for (const arm of fn[1].matchAll(/"([^"]+)"\s*=>\s*(&\[[\s\S]*?\]),/g)) {
        out.set(
            arm[1],
            Object.entries(srvPairs(arm[2])).map(([name, version]) => ({ name, version }))
        );
    }
    return out;
}

/** `prompt_arg_flag`: the one provider id and flag it names. */
function srvPromptArgFlags(): Map<string, string> {
    const fn = providersRs.match(/pub fn prompt_arg_flag\(&self\)[^{]*\{([\s\S]*?)\n    \}/);
    if (!fn) throw new Error("prompt_arg_flag not found in providers.rs");
    const out = new Map<string, string>();
    for (const m of fn[1].matchAll(/self\.id == "([^"]+)"\)\.then_some\("([^"]+)"\)/g)) out.set(m[1], m[2]);
    if (out.size === 0) throw new Error("prompt_arg_flag names no provider: update this test with its new shape");
    return out;
}

describe("srv ProviderConfig and the frontend catalog agree", () => {
    const companions = srvCompanions();
    const promptFlags = srvPromptArgFlags();

    it("both define the same providers", () => {
        expect(statics.map((s) => s.id).sort()).toEqual(Object.keys(PROVIDERS).sort());
    });

    it("srv names no companion package or prompt flag for a provider the catalog lacks", () => {
        for (const id of [...companions.keys(), ...promptFlags.keys()]) {
            expect(PROVIDERS[id], `srv names ${id}, which the catalog does not define`).toBeTruthy();
        }
    });

    for (const p of statics) {
        describe(p.id, () => {
            const ts = PROVIDERS[p.id];

            it("command and controller", () => {
                expect(ts.cliCommand, "cliCommand").toBe(srvString(srvField(p, "cli_command")));
                const variant = srvField(p, "controller_type").match(/^ControllerType::(\w+)$/)?.[1] ?? "";
                expect(CONTROLLER[variant], `controller_type ${variant} has no catalog name: add it to CONTROLLER`).toBeTruthy();
                expect(ts.controllerType, "controllerType").toBe(CONTROLLER[variant]);
            });

            it("resume and session id", () => {
                const flag = srvOptString(srvField(p, "resume_flag"));
                expect(ts.resumeFlag ?? null, "resumeFlag").toBe(flag);
                // srv's `resume_strategy_str`: codex resumes through `exec resume`, others by flag or not at all.
                const strategy = p.id === "codex" ? "codex-exec" : flag !== null ? "flag" : "none";
                expect(ts.resumeStrategy ?? (ts.resumeFlag ? "flag" : "none"), "resumeStrategy").toBe(strategy);
                expect(ts.sessionIdField, "sessionIdField").toBe(srvString(srvField(p, "session_id_field")));
                expect(ts.styledOutputFormat, "styledOutputFormat").toBe(srvString(srvField(p, "styled_output_format")));
            });

            it("auth directory and environment", () => {
                expect(ts.authConfigDirEnvVar, "authConfigDirEnvVar").toBe(srvString(srvField(p, "auth_config_dir_env_var")));
                expect(ts.authDirName, "authDirName").toBe(srvString(srvField(p, "auth_dir_name")));
                expect(ts.authExtraEnv ?? {}, "authExtraEnv").toEqual(srvPairs(srvField(p, "auth_extra_env")));
                expect(ts.unsetEnv ?? [], "unsetEnv").toEqual(srvStringList(srvField(p, "unset_env")));
            });

            it("npm package, pin and companions", () => {
                expect(ts.npmPackage, "npmPackage").toBe(srvString(srvField(p, "npm_package")));
                expect(ts.pinnedVersion, "pinnedVersion").toBe(srvString(srvField(p, "pinned_version")));
                expect(ts.companionNpmPackages ?? [], "companionNpmPackages").toEqual(companions.get(p.id) ?? []);
            });

            it("vendor, startup file and prompt flag", () => {
                expect(ts.baseUrlEnvVar ?? null, "baseUrlEnvVar").toBe(srvOptString(srvField(p, "base_url_env_var")));
                expect(ts.supportedVendors ?? [], "supportedVendors").toEqual(srvStringList(srvField(p, "supported_vendors")));
                expect(ts.startupInstructionsFilename ?? null, "startupInstructionsFilename").toBe(
                    srvOptString(srvField(p, "startup_instructions_filename"))
                );
                expect(ts.promptArgFlag ?? null, "promptArgFlag").toBe(promptFlags.get(p.id) ?? null);
            });
        });
    }
});
