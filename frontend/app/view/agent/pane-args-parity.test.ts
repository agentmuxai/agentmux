// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The strip and the process must agree — enforced over EVERY provider.
 *
 * The runtime strip (model · effort · mode) renders `agent:runtime` block meta.
 * The process runs on `cmd:args`. They are separate records written by
 * separate code, and they drifted: a resumed agent was launched with a
 * `cmd:args` that had no `--model`/`--effort` while the strip read Sonnet, so
 * it ran on the CLI default (Opus 5.5) for its whole life.
 * docs/retro/RETRO_RESUMED_AGENT_SPAWNS_WITHOUT_RUNTIME_FLAGS_2026_09_30.md.
 *
 * Every claim below is stated for all providers in the catalog at once, so a
 * provider added later is checked without anyone remembering to add a case.
 *
 * The KNOWN_* sets are ratchets, not approvals. Each lists a gap that exists
 * today (docs/reports/REPORT_AGENT_RUNTIME_BINDINGS_2026_09_30.md §4). A test
 * fails when a gap closes without its entry being removed, and when a new gap
 * appears without one being added — so the list can only shrink on purpose.
 */

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/logger", () => ({ Logger: { warn: () => {} } }));
vi.mock("@/app/store/global", () => ({ getApi: () => ({}) }));

import { resolveInitialRuntimeConfig } from "./agent-launch-env";
import { buildPaneArgs, buildRuntimeArgs, providerSupportsModelFlag } from "./buildRuntimeArgs";
import { selectLaunchArgs } from "./launch-args";
import { getProviderList } from "./providers";
import { DEFAULT_RUNTIME_CONFIG, type AgentRuntimeConfig } from "./types";

const providers = getProviderList();
const MODES = ["host", "container"] as const;
const BYPASS_FLAG = "--dangerously-skip-permissions";

/** The value following `flag`, or undefined when absent. */
function flagValue(args: string[], flag: string): string | undefined {
    const i = args.indexOf(flag);
    return i >= 0 ? args[i + 1] : undefined;
}
const count = (args: string[], flag: string) => args.filter((a) => a === flag).length;

/** Providers whose rebuilt args gain `--dangerously-skip-permissions` although
 *  their catalog args never declared it. `buildRuntimeArgs` only special-cases
 *  kimi/gemini/qwen (`--yolo`) and codex (baked in); everything else falls into
 *  the Claude branch on every send. Whether each CLI tolerates the flag is
 *  unverified. Report §4 G1. */
const KNOWN_STRAY_BYPASS_FLAG = new Set(["muxcode", "openclaw", "copilot", "pi"]);

/** Providers whose catalog `--yolo` is stripped and not restored. Report §4 G1. */
const KNOWN_YOLO_DROPPED = new Set<string>([]);

/** Providers that list `models` but get no `--model` (menu offers a choice that
 *  is never applied). Report §4 G2. */
const KNOWN_MODELS_NOT_APPLIED = new Set<string>([]);

describe("pane args — every provider, every mode", () => {
    for (const p of providers) {
        for (const mode of MODES) {
            describe(`${p.id} (${mode})`, () => {
                const runtime: AgentRuntimeConfig = {
                    ...DEFAULT_RUNTIME_CONFIG,
                    model: p.models?.find((m) => m.default)?.value ?? DEFAULT_RUNTIME_CONFIG.model,
                };
                const args = buildPaneArgs(p, mode, runtime, "");

                it("never repeats a runtime flag", () => {
                    for (const f of ["--model", "--effort", "--permission-mode", BYPASS_FLAG, "--yolo"]) {
                        expect(count(args, f), `${f} in ${JSON.stringify(args)}`).toBeLessThanOrEqual(1);
                    }
                });

                it("is stable when rebuilt from its own output (the per-send rebuild is idempotent)", () => {
                    expect(buildRuntimeArgs(args, runtime, p.id)).toEqual(args);
                });

                it("never carries the launch-only --fork-session", () => {
                    expect(args).not.toContain("--fork-session");
                });

                it("keeps codex's stdin marker last", () => {
                    if (p.id === "codex") expect(args[args.length - 1]).toBe("-");
                });
            });
        }
    }
});

describe("the strip and the args agree (Claude)", () => {
    const claude = providers.find((p) => p.id === "claude")!;
    const models = (claude.models ?? []).map((m) => m.value);
    const efforts = ["low", "medium", "high", "xhigh", "max"] as const;

    it("the catalog offers models to check", () => {
        expect(models.length).toBeGreaterThan(0);
    });

    for (const mode of MODES) {
        it(`${mode}: --model and --effort equal agent:runtime for every model × effort`, () => {
            for (const model of models) {
                for (const effort of efforts) {
                    const args = buildPaneArgs(claude, mode, { permissionMode: "bypass", model, effort }, "");
                    expect(flagValue(args, "--model"), `model ${model}`).toBe(model);
                    // --effort 400s on Haiku, so the strip's effort is not applied there (gap G4).
                    if (model === "haiku") expect(args).not.toContain("--effort");
                    else expect(flagValue(args, "--effort"), `${model}/${effort}`).toBe(effort);
                }
            }
        });
    }

    it("a persistent agent is never handed the bypass flag (it would drop can_use_tool)", () => {
        const args = buildPaneArgs(claude, "host", { ...DEFAULT_RUNTIME_CONFIG, permissionMode: "bypass" }, "");
        expect(args).not.toContain(BYPASS_FLAG);
        expect(flagValue(args, "--permission-mode")).toBe("default");
    });

    it("a fresh launch's args match the runtime the launch writes to block meta", () => {
        // launchAgentDefinition writes exactly these two values side by side.
        const runtime = resolveInitialRuntimeConfig(undefined, claude.models);
        const args = buildPaneArgs(claude, "host", runtime, "");
        expect(flagValue(args, "--model")).toBe(runtime.model);
        expect(flagValue(args, "--effort")).toBe(runtime.effort);
    });

    it("an explicit model chosen at launch reaches the process", () => {
        const runtime = resolveInitialRuntimeConfig("opus", claude.models);
        expect(flagValue(buildPaneArgs(claude, "host", runtime, ""), "--model")).toBe("opus");
    });
});

describe("a model the menu offers is a model the process gets", () => {
    for (const p of providers.filter((x) => (x.models?.length ?? 0) > 0)) {
        const applied = providerSupportsModelFlag(p.id);
        const known = KNOWN_MODELS_NOT_APPLIED.has(p.id);
        it(`${p.id}: ${applied ? "gets --model" : known ? "KNOWN GAP — lists models, applies none" : "lists models but applies none"}`, () => {
            const runtime = resolveInitialRuntimeConfig(undefined, p.models);
            const args = buildPaneArgs(p, "host", runtime, "");
            if (applied) {
                const picked = flagValue(args, "--model");
                expect(picked, JSON.stringify(args)).toBeDefined();
                // codex may fall back to its catalog default for an unknown stored value
                expect(p.models!.map((m) => m.value)).toContain(picked);
            } else {
                expect(args).not.toContain("--model");
                expect(known, `${p.id} lists models but buildRuntimeArgs never applies one; fix it or add it to KNOWN_MODELS_NOT_APPLIED`).toBe(true);
            }
        });
    }

    it("KNOWN_MODELS_NOT_APPLIED names only providers that still have the gap", () => {
        for (const id of KNOWN_MODELS_NOT_APPLIED) {
            const p = providers.find((x) => x.id === id);
            expect(p, `${id} left the catalog; remove it from the set`).toBeDefined();
            expect(providerSupportsModelFlag(id) && (p!.models?.length ?? 0) > 0, `${id} is fixed; remove it from the set`).toBe(false);
        }
    });
});

describe("permission flags stay in the provider's own vocabulary", () => {
    it("the set of providers that gain a flag their catalog never declared is exactly the known one", () => {
        const gained = providers
            .filter((p) => {
                const base = selectLaunchArgs(p, "host");
                const built = buildPaneArgs(p, "host", DEFAULT_RUNTIME_CONFIG, "");
                return built.includes(BYPASS_FLAG) && !base.includes(BYPASS_FLAG);
            })
            .map((p) => p.id)
            .sort();
        expect(gained).toEqual([...KNOWN_STRAY_BYPASS_FLAG].sort());
    });

    it("the set of providers whose catalog --yolo is dropped is exactly the known one", () => {
        const dropped = providers
            .filter((p) => {
                const base = selectLaunchArgs(p, "host");
                const built = buildPaneArgs(p, "host", DEFAULT_RUNTIME_CONFIG, "");
                return base.includes("--yolo") && !built.includes("--yolo");
            })
            .map((p) => p.id)
            .sort();
        expect(dropped).toEqual([...KNOWN_YOLO_DROPPED].sort());
    });
});

describe("one composition only", () => {
    /** Non-test source files under frontend/ that call buildRuntimeArgs directly.
     *  Launch, the per-send rebuild and a runtime change must all go through
     *  buildPaneArgs, so they cannot drift apart again. */
    function sources(dir: string): string[] {
        return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
            const p = join(dir, e.name);
            if (e.isDirectory()) return e.name === "node_modules" ? [] : sources(p);
            return /\.(ts|tsx)$/.test(e.name) && !/\.test\.tsx?$/.test(e.name) ? [p] : [];
        });
    }

    const all = sources(join(process.cwd(), "frontend"));
    const callers = (re: RegExp, allowed: string[]) =>
        all.filter((f) => !allowed.some((a) => f.endsWith(a))).filter((f) => re.test(readFileSync(f, "utf8")));

    it("only buildRuntimeArgs.ts calls buildRuntimeArgs()", () => {
        expect(callers(/\bbuildRuntimeArgs\(/, ["buildRuntimeArgs.ts"])).toEqual([]);
    });

    /** Every runtime change goes through `patchRuntime`'s per-pane queue. A writer
     *  that calls `applyRuntimeChange` itself builds its config from the block's
     *  meta at click time and can undo a change still in flight. */
    it("only runtime-apply.ts calls applyRuntimeChange()", () => {
        expect(callers(/\bapplyRuntimeChange\(/, ["runtime-apply.ts"])).toEqual([]);
    });

    /** `cmd:args` built from `selectLaunchArgs` alone has no --model/--effort.
     *  That is the exact shape of the launch bug: launchAgentDefinition did this. */
    it("only buildPaneArgs reads the catalog's base args", () => {
        expect(callers(/\bselectLaunchArgs\(/, ["buildRuntimeArgs.ts", "launch-args.ts"])).toEqual([]);
    });
});
