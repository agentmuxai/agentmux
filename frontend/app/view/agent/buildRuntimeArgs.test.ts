// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { buildPaneArgs, buildRuntimeArgs } from "./buildRuntimeArgs";
import { getProvider, replaceProviderModels } from "./providers";
import type { AgentRuntimeConfig } from "./types";

const CLAUDE_PROVIDER = getProvider("claude")!;

// Base args as declared in providers/index.ts (launchArgs).
const CODEX_BASE = ["exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "-"];
const CLAUDE_BASE = [
    "-p",
    "--output-format",
    "stream-json",
    "--verbose",
    "--include-partial-messages",
    "--dangerously-skip-permissions",
];

// Claude's real persistentLaunchArgs (providers/catalog.ts, kept in sync with
// `static CLAUDE` in crates/srv/src/backend/providers.rs). Note the control-
// protocol pair at the end — this is what runtime-apply.ts rebuilds on every
// /model, /effort or permission-mode change to a running persistent agent.
const CLAUDE_PERSISTENT_BASE = [
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--include-partial-messages",
    "--permission-prompt-tool",
    "stdio",
    "--permission-mode",
    "default",
];

const cfg = (over: Partial<AgentRuntimeConfig> = {}): AgentRuntimeConfig => ({
    permissionMode: "bypass",
    model: "sonnet",
    effort: "medium",
    ...over,
});

describe("buildRuntimeArgs", () => {
    describe("codex (regression: launch-DOA from Claude-shaped args)", () => {
        const CODEX_FIXED = [
            "exec",
            "--json",
            "--dangerously-bypass-approvals-and-sandbox",
            "--model",
            "gpt-6-astra",
            "-",
        ];

        it("no Claude permission flag / Claude model; inserts a gpt-5.x model BEFORE the `-` positional", () => {
            const out = buildRuntimeArgs(CODEX_BASE, cfg(), "codex");
            expect(out).toEqual(CODEX_FIXED);
            // The exact things that killed the codex process before the fix:
            expect(out).not.toContain("--dangerously-skip-permissions");
            expect(out).not.toContain("sonnet");
            // model is the catalog's ChatGPT-account default, not a Claude alias
            expect(out[out.indexOf("--model") + 1]).toBe("gpt-6-astra");
            // codex `exec` reads the prompt from the trailing positional `-`; the
            // model flag must sit before it, and nothing may follow it.
            expect(out[out.length - 1]).toBe("-");
        });

        it("falls back to the codex default when a carried-over Claude model is stored", () => {
            // A pane created before per-provider models may have `model:"opus"`.
            const out = buildRuntimeArgs(CODEX_BASE, cfg({ model: "opus" }), "codex");
            expect(out).toEqual(CODEX_FIXED);
            expect(out).not.toContain("opus");
        });

        it("honors a user-picked codex model", () => {
            const out = buildRuntimeArgs(CODEX_BASE, cfg({ model: "gpt-5.6-sol" }), "codex");
            expect(out[out.indexOf("--model") + 1]).toBe("gpt-5.6-sol");
            expect(out[out.length - 1]).toBe("-"); // still before the positional
        });

        it("replaces a pick the model list no longer offers with its default", () => {
            const out = buildRuntimeArgs(CODEX_BASE, cfg({ model: "gpt-5.5" }), "codex");
            expect(out[out.indexOf("--model") + 1]).toBe("gpt-6-astra");
        });

        it("names no model when none was chosen and the account's list is unknown: the CLI picks", () => {
            const out = buildRuntimeArgs(CODEX_BASE, cfg({ model: "" }), "codex");
            expect(out).not.toContain("--model");
            expect(out[out.length - 1]).toBe("-");
        });
    });

    it("never gives Claude a level only Codex takes, remembered from a Codex pick", () => {
        const out = buildRuntimeArgs(CLAUDE_BASE, cfg({ model: "opus", effort: "ultra" }), "claude");
        expect(out).not.toContain("--effort");
        expect(out).not.toContain("ultra");
        expect(buildRuntimeArgs(CLAUDE_BASE, cfg({ model: "opus", effort: "max" }), "claude")).toContain("max");
    });

    // Config overrides, since `codex exec resume` (every turn after the first)
    // takes `-c` but has no `--sandbox`; verified against codex-cli 0.154.0.
    describe("codex effort and mode", () => {
        afterEach(() => replaceProviderModels("codex", []));
        const listAstra = () =>
            replaceProviderModels("codex", [
                { value: "gpt-6-astra", label: "GPT-6-Astra", efforts: ["low", "medium", "high", "ultra"], defaultEffort: "low" },
            ]);

        it("passes a level the model takes as model_reasoning_effort, before the `-` positional", () => {
            listAstra();
            const out = buildRuntimeArgs(CODEX_BASE, cfg({ model: "gpt-6-astra", effort: "ultra" }), "codex");
            expect(out).toEqual([
                "exec",
                "--json",
                "--dangerously-bypass-approvals-and-sandbox",
                "--model",
                "gpt-6-astra",
                "-c",
                "model_reasoning_effort=ultra",
                "-",
            ]);
            expect(out).not.toContain("--effort");
        });

        it("leaves out a level the model doesn't take, so the CLI runs the model's own default", () => {
            listAstra();
            const out = buildRuntimeArgs(CODEX_BASE, cfg({ model: "gpt-6-astra", effort: "max" }), "codex");
            expect(out.join(" ")).not.toContain("model_reasoning_effort");
            // Nor while the account's list (and so the model's levels) is unknown.
            replaceProviderModels("codex", []);
            const unknown = buildRuntimeArgs(CODEX_BASE, cfg({ model: "gpt-6-astra", effort: "high" }), "codex");
            expect(unknown.join(" ")).not.toContain("model_reasoning_effort");
        });

        it("runs Workspace write and Read only in Codex's sandbox, without the bypass flag", () => {
            const write = buildRuntimeArgs(CODEX_BASE, cfg({ permissionMode: "acceptEdits" }), "codex");
            expect(write).not.toContain("--dangerously-bypass-approvals-and-sandbox");
            expect(write.slice(-5)).toEqual(["-c", "sandbox_mode=workspace-write", "-c", "approval_policy=never", "-"]);
            const read = buildRuntimeArgs(CODEX_BASE, cfg({ permissionMode: "plan" }), "codex");
            expect(read.slice(-5)).toEqual(["-c", "sandbox_mode=read-only", "-c", "approval_policy=never", "-"]);
            // Full access is the base args as they are.
            expect(buildRuntimeArgs(CODEX_BASE, cfg({ permissionMode: "bypass" }), "codex")).toContain(
                "--dangerously-bypass-approvals-and-sandbox",
            );
        });
    });

    describe("claude (unchanged)", () => {
        it("applies bypass permission + model + effort", () => {
            const out = buildRuntimeArgs(CLAUDE_BASE, cfg(), "claude");
            expect(out).toContain("--dangerously-skip-permissions");
            expect(out[out.indexOf("--model") + 1]).toBe("sonnet");
            expect(out[out.indexOf("--effort") + 1]).toBe("medium");
        });

        it("maps a non-bypass mode to --permission-mode and strips the stale bypass flag", () => {
            const out = buildRuntimeArgs(CLAUDE_BASE, cfg({ permissionMode: "plan" }), "claude");
            expect(out).not.toContain("--dangerously-skip-permissions");
            expect(out[out.indexOf("--permission-mode") + 1]).toBe("plan");
        });

        it("omits --effort on Haiku (effort 400s on Haiku 4.5) but keeps --model", () => {
            const out = buildRuntimeArgs(CLAUDE_BASE, cfg({ model: "haiku" }), "claude");
            expect(out[out.indexOf("--model") + 1]).toBe("haiku");
            expect(out).not.toContain("--effort");
        });

        it("decides --effort on the model the process will RUN: a definition's own Haiku (Codex P1 on #4152)", () => {
            // Runtime says Sonnet (takes effort); the definition's provider_flags
            // are appended after and override to Haiku (does not). Sending --effort
            // here is HTTP 400 on every turn.
            const haiku = buildPaneArgs(CLAUDE_PROVIDER, "host", cfg({ model: "sonnet" }), "--model claude-haiku-4-5-20251001");
            expect(haiku).not.toContain("--effort");
            expect(haiku.filter((a) => a === "--model")).toHaveLength(2); // the runtime's, then the definition's, which wins
            // …and the other way round: a definition's Opus under a runtime Haiku DOES take effort.
            expect(buildPaneArgs(CLAUDE_PROVIDER, "host", cfg({ model: "haiku" }), "--model opus")).toContain("--effort");
            // no override in the flags: the runtime's model decides, as before
            expect(buildPaneArgs(CLAUDE_PROVIDER, "host", cfg({ model: "sonnet" }), "--add-dir /tmp")).toContain("--effort");
        });

        it("never sends an --effort the running Haiku rejects, even one the definition itself pinned", () => {
            // The definition says Haiku AND an effort: appended after the runtime's
            // flags, it would be HTTP 400 on every turn.
            for (const flags of ["--model haiku --effort max", "--model claude-haiku-4-5-20251001 --effort=low"]) {
                const out = buildPaneArgs(CLAUDE_PROVIDER, "host", cfg({ model: "sonnet" }), flags);
                expect(out, flags).not.toContain("--effort");
                expect(out.some((a) => a.startsWith("--effort=")), flags).toBe(false);
            }
            // a model that takes effort keeps the definition's own
            expect(buildPaneArgs(CLAUDE_PROVIDER, "host", cfg({ model: "sonnet" }), "--model opus --effort max")).toContain("max");
        });

        it("leaves another provider's own --effort alone (muxreview P2 on #4161): only Claude rejects it on Haiku", () => {
            for (const id of ["codex", "gemini", "kimi", "qwen"]) {
                const p = getProvider(id)!;
                const out = buildPaneArgs(p, "host", cfg({ model: "haiku" }), "--effort high --add-dir /tmp");
                expect(out, id).toContain("--effort");
                expect(out[out.indexOf("--effort") + 1], id).toBe("high");
            }
        });

        it("omits --effort for a CONCRETE Haiku id too (it used to match only the alias)", () => {
            const out = buildRuntimeArgs(CLAUDE_BASE, cfg({ model: "claude-haiku-4-5-20251001" }), "claude");
            expect(out[out.indexOf("--model") + 1]).toBe("claude-haiku-4-5-20251001");
            expect(out).not.toContain("--effort");
        });
    });

    describe("gemini", () => {
        it("uses --yolo for a non-default mode; no Claude bypass flag and no Claude --model", () => {
            const out = buildRuntimeArgs(["--output-format", "stream-json", "-p", ""], cfg(), "gemini");
            expect(out).toContain("--yolo");
            expect(out).not.toContain("--dangerously-skip-permissions");
            // gemini no longer receives the Claude-named ModelChoice (it rejects it)
            expect(out).not.toContain("--model");
            expect(out).not.toContain("sonnet");
        });

        it("no --yolo when the permission mode is default", () => {
            const out = buildRuntimeArgs(
                ["--output-format", "stream-json", "-p", ""],
                cfg({ permissionMode: "default" }),
                "gemini",
            );
            expect(out).not.toContain("--yolo");
        });
    });

    describe("qwen (Gemini-CLI fork — same --yolo permission model)", () => {
        // qwen's real launchArgs include --yolo; PERMISSION_STRIP removes it,
        // then a non-default mode must re-add it (the regression this guards —
        // without "qwen" in the branch it fell through to Claude-style flags).
        const QWEN_BASE = ["--output-format", "stream-json", "--yolo", "-p", ""];

        it("strips then re-adds --yolo for a non-default mode; no Claude bypass flag / no --model leak", () => {
            const out = buildRuntimeArgs(QWEN_BASE, cfg(), "qwen");
            expect(out).toContain("--yolo");
            expect(out).not.toContain("--dangerously-skip-permissions");
            expect(out).not.toContain("--model");
            expect(out).not.toContain("sonnet");
        });

        it("no --yolo when the permission mode is default", () => {
            const out = buildRuntimeArgs(QWEN_BASE, cfg({ permissionMode: "default" }), "qwen");
            expect(out).not.toContain("--yolo");
        });
    });

    // Both provider catalogs state this in capitals: --dangerously-skip-
    // permissions DISABLES canUseTool routing, so it must never reach a
    // control-protocol agent. Nothing enforced it, and this function is what
    // rebuilds a running agent's args on every runtime change — so the FIRST
    // /model on a persistent Claude agent used to switch the CLI out of the
    // control protocol and leave AskUserQuestion unanswerable from then on.
    describe("claude persistent (Agent SDK control protocol)", () => {
        it("never hands a control-protocol agent the bypass flag", () => {
            const out = buildRuntimeArgs(CLAUDE_PERSISTENT_BASE, cfg(), "claude");
            expect(out).not.toContain("--dangerously-skip-permissions");
        });

        it("keeps the control-protocol transport intact", () => {
            const out = buildRuntimeArgs(CLAUDE_PERSISTENT_BASE, cfg(), "claude");
            expect(out).toContain("--permission-prompt-tool");
            expect(out).toContain("stdio");
            // bypass maps onto default: srv's ControlChannel auto-allows every
            // tool but AskUserQuestion, so the yolo UX is unchanged.
            expect(out).toContain("--permission-mode");
            expect(out[out.indexOf("--permission-mode") + 1]).toBe("default");
        });

        it("still honours an explicitly chosen non-bypass mode", () => {
            const out = buildRuntimeArgs(
                CLAUDE_PERSISTENT_BASE,
                cfg({ permissionMode: "plan" }),
                "claude",
            );
            expect(out[out.indexOf("--permission-mode") + 1]).toBe("plan");
            expect(out).not.toContain("--dangerously-skip-permissions");
        });

        it("is idempotent — rebuilding its own output does not drift", () => {
            // runtime-apply.ts writes the result to cmd:args, and the next
            // change rebuilds from THAT, not from the catalog.
            const once = buildRuntimeArgs(CLAUDE_PERSISTENT_BASE, cfg(), "claude");
            const twice = buildRuntimeArgs(once, cfg(), "claude");
            expect(twice).toEqual(once);
        });

        it("leaves the non-persistent claude path alone", () => {
            // CLAUDE_BASE carries --dangerously-skip-permissions and no
            // control-protocol flag; bypass must still mean bypass there.
            const out = buildRuntimeArgs(CLAUDE_BASE, cfg(), "claude");
            expect(out).toContain("--dangerously-skip-permissions");
        });
    });
});

// agy's own flags (SPEC_ANTIGRAVITY_HARNESS_REAL_CLI_2026_10_06.md): no
// --yolo or --permission-mode, and --model takes one of `agy models`' ids.
describe("buildRuntimeArgs — antigravity", () => {
    const base = getProvider("antigravity")!.launchArgs;
    const run = (permissionMode: AgentRuntimeConfig["permissionMode"], model = "gemini-3.1-pro-high") =>
        buildRuntimeArgs(base, { permissionMode, model, effort: "high" }, "antigravity");

    it("maps each permission mode to agy's own flag", () => {
        expect(run("bypass")).toEqual([
            "--output-format", "stream-json", "--dangerously-skip-permissions", "--model", "gemini-3.1-pro-high",
        ]);
        expect(run("plan")).toEqual(["--output-format", "stream-json", "--mode", "plan", "--model", "gemini-3.1-pro-high"]);
        expect(run("acceptEdits")).toContain("accept-edits");
        // agy has no "auto"; it and "default" leave agy's own review mode.
        for (const mode of ["default", "auto"] as const) {
            const args = run(mode);
            expect(args).not.toContain("--dangerously-skip-permissions");
            expect(args).not.toContain("--mode");
            expect(args).not.toContain("--permission-mode");
        }
    });

    it("never passes a Claude alias or --effort to agy", () => {
        const args = run("bypass", "sonnet");
        expect(args[args.indexOf("--model") + 1]).toBe("gemini-3.8-flash-medium");
        expect(args).not.toContain("--effort");
    });

    it("puts no prompt placeholder in the args: the backend appends -p <prompt>", () => {
        expect(run("bypass")).not.toContain("-p");
        expect(getProvider("antigravity")!.promptArgFlag).toBe("-p");
    });
});

describe("buildRuntimeArgs — ACP, MSP and terminal-tier agents", () => {
    // Permission is asked inside ACP (session/request_permission) or MSP
    // (approval/request), or in a terminal-tier CLI's own UI, so the launch
    // args stay exactly the catalog's in every mode.
    for (const id of ["copilot", "openclaw", "pi", "kilo", "cline", "opencode", "goose", "grok", "dsh", "aider", "crush", "muse"]) {
        it(`${id}: no permission, model or effort flag in any mode`, () => {
            const base = getProvider(id)!.launchArgs;
            for (const permissionMode of ["default", "auto", "acceptEdits", "plan", "bypass"] as const) {
                expect(buildRuntimeArgs(base, { permissionMode, model: "opus", effort: "high" }, id)).toEqual(base);
            }
        });
    }
});
