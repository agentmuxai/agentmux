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
            "gpt-5.4",
            "-",
        ];

        it("no Claude permission flag / Claude model; inserts a gpt-5.x model BEFORE the `-` positional", () => {
            const out = buildRuntimeArgs(CODEX_BASE, cfg(), "codex");
            expect(out).toEqual(CODEX_FIXED);
            // The exact things that killed the codex process before the fix:
            expect(out).not.toContain("--dangerously-skip-permissions");
            expect(out).not.toContain("sonnet");
            // model is a ChatGPT-account-supported gpt-5.x, not the Claude ModelChoice
            expect(out[out.indexOf("--model") + 1]).toBe("gpt-5.4");
            // codex `exec` reads the prompt from the trailing positional `-`; the
            // model flag must sit before it, and nothing may follow it.
            expect(out[out.length - 1]).toBe("-");
        });

        it("ignores the Claude-shaped runtime overrides (permission mode + ModelChoice)", () => {
            const out = buildRuntimeArgs(CODEX_BASE, cfg({ permissionMode: "plan", model: "opus" }), "codex");
            expect(out).toEqual(CODEX_FIXED);
            expect(out).not.toContain("opus");
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
    });

    describe("gemini (unchanged)", () => {
        it("uses --yolo for a non-default permission mode, never the Claude bypass flag", () => {
            const out = buildRuntimeArgs(["--output-format", "stream-json", "-p", ""], cfg(), "gemini");
            expect(out).toContain("--yolo");
            expect(out).not.toContain("--dangerously-skip-permissions");
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
});
