// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildRuntimeArgs } from "./buildRuntimeArgs";
import type { AgentRuntimeConfig } from "./types";

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
// `static CLAUDE` in agentmux-srv/src/backend/providers.rs). Note the control-
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
            "gpt-5.5",
            "-",
        ];

        it("no Claude permission flag / Claude model; inserts a gpt-5.x model BEFORE the `-` positional", () => {
            const out = buildRuntimeArgs(CODEX_BASE, cfg(), "codex");
            expect(out).toEqual(CODEX_FIXED);
            // The exact things that killed the codex process before the fix:
            expect(out).not.toContain("--dangerously-skip-permissions");
            expect(out).not.toContain("sonnet");
            // model is a ChatGPT-account-supported gpt-5.x default, not a Claude alias
            expect(out[out.indexOf("--model") + 1]).toBe("gpt-5.5");
            // codex `exec` reads the prompt from the trailing positional `-`; the
            // model flag must sit before it, and nothing may follow it.
            expect(out[out.length - 1]).toBe("-");
        });

        it("falls back to the codex default when a carried-over Claude model is stored", () => {
            // A pane created before per-provider models may have `model:"opus"`.
            const out = buildRuntimeArgs(CODEX_BASE, cfg({ permissionMode: "plan", model: "opus" }), "codex");
            expect(out).toEqual(CODEX_FIXED);
            expect(out).not.toContain("opus");
        });

        it("honors a user-picked codex model", () => {
            const out = buildRuntimeArgs(CODEX_BASE, cfg({ model: "gpt-5.4" }), "codex");
            expect(out[out.indexOf("--model") + 1]).toBe("gpt-5.4");
            expect(out[out.length - 1]).toBe("-"); // still before the positional
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
