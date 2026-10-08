// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const patchRuntime = vi.fn();
// A plain function for the failure case: vitest reports what a vi.fn
// implementation rejects as a test error even when the caller handles it.
let failWith: Error | null = null;
vi.mock("../../runtime-apply", () => ({
    patchRuntime: (...a: unknown[]) => {
        if (failWith) return Promise.reject(failWith);
        return patchRuntime(...a);
    },
}));

import { RUNTIME_COMMANDS } from "./runtime";
import type { SlashCommandContext } from "../types";

const effort = RUNTIME_COMMANDS.find((c) => c.name === "effort")!;
const ctx = (providerId: string) =>
    ({ blockId: "b1", provider: () => ({ id: providerId }), block: () => ({ meta: {} }) }) as unknown as SlashCommandContext;

beforeEach(() => {
    patchRuntime.mockReset();
    failWith = null;
});

describe("/effort", () => {
    it("says it applies when it does", async () => {
        patchRuntime.mockResolvedValue({ permissionMode: "bypass", model: "sonnet", effort: "max" });
        const r = await effort.handler(ctx("claude"), "max");
        expect(r).toEqual({ kind: "ok", message: "effort set to max (applies to next turn)" });
    });

    it("says plainly when it changes nothing now: Haiku, by alias or concrete id", async () => {
        for (const model of ["haiku", "claude-haiku-4-5-20251001"]) {
            patchRuntime.mockResolvedValue({ permissionMode: "bypass", model, effort: "max" });
            const r = (await effort.handler(ctx("claude"), "max")) as { message: string };
            expect(r.message, model).toMatch(/recorded as max, but it has no effect now/);
            expect(r.message, model).toMatch(/Haiku/);
        }
    });

    it("says so for a provider with no effort setting", async () => {
        patchRuntime.mockResolvedValue({ permissionMode: "bypass", model: "gpt-5.5", effort: "low" });
        const r = (await effort.handler(ctx("codex"), "low")) as { message: string };
        expect(r.message).toMatch(/no effect now — codex has no effort setting/);
    });

    it("still reports a failure to apply", async () => {
        failWith = new Error("resync refused");
        const r = (await effort.handler(ctx("claude"), "low")) as { kind: string; message: string };
        expect(r.kind).toBe("error");
        expect(r.message).toContain("resync refused");
    });
});

describe("/runtime and the choice lists show what the agent really runs", () => {
    const meta = {
        "agent:runtime": { model: "sonnet", permissionMode: "bypass", effort: "high" },
        "agent:provider_flags": "--model opus --effort max",
    };
    const c = { blockId: "b1", provider: () => ({ id: "claude", models: [{ value: "sonnet", label: "Sonnet" }, { value: "opus", label: "Opus" }] }), block: () => ({ meta }) } as unknown as SlashCommandContext;

    it("/runtime prints the definition's model and effort", async () => {
        const cmd = RUNTIME_COMMANDS.find((x) => x.name === "runtime")!;
        const r = (await cmd.handler(c, "")) as { message: string };
        expect(r.message).toContain("model: opus");
        expect(r.message).toContain("effort: max");
        expect(r.message).not.toContain("model: sonnet");
    });

    it("/model marks the model that runs as the current one", () => {
        const cmd = RUNTIME_COMMANDS.find((x) => x.name === "model")!;
        const choices = (cmd.arg as any).choices(c) as { value: string; current?: boolean }[];
        expect(choices.find((x) => x.current)?.value).toBe("opus");
    });
});

describe("/permission-mode and /runtime show the mode a definition pins (muxreview P2 on #4161)", () => {
    const meta = {
        "agent:runtime": { model: "sonnet", permissionMode: "bypass", effort: "high" },
        "agent:provider_flags": "--permission-mode plan",
    };
    const c = { blockId: "b1", provider: () => ({ id: "claude", controllerType: "persistent", launchArgs: [], persistentLaunchArgs: [] }), block: () => ({ meta }) } as unknown as SlashCommandContext;

    it("/permission-mode marks the mode that runs as current", () => {
        const cmd = RUNTIME_COMMANDS.find((x) => x.name === "permission-mode")!;
        const choices = (cmd.arg as any).choices(c) as { value: string; current?: boolean }[];
        expect(choices.find((x) => x.current)?.value).toBe("plan");
    });
    it("/runtime prints it", async () => {
        const cmd = RUNTIME_COMMANDS.find((x) => x.name === "runtime")!;
        const r = (await cmd.handler(c, "")) as { message: string };
        expect(r.message).toContain("permission: plan");
    });
});

describe("/permission-mode choices", () => {
    const perm = RUNTIME_COMMANDS.find((c) => c.name === "permission-mode")!;
    const choices = (agentMode: string) =>
        (perm.arg as any).choices({
            blockId: "b1",
            provider: () => ({ id: "claude", controllerType: "persistent", launchArgs: ["-p"], persistentLaunchArgs: ["--x"] }),
            block: () => ({ meta: { agentMode } }),
        }) as { value: string; description: string }[];

    it("a persistent agent: says what actually happens", () => {
        const d = Object.fromEntries(choices("host").map((c) => [c.value, c.description]));
        expect(d.default).toMatch(/allowed automatically/);
        expect(d.plan).toMatch(/does NOT stop edits/);
        expect(d.default).not.toMatch(/Standard permission prompts/);
    });

    it("a container agent: the CLI refuses what the mode does not allow; Plan keeps its own description", () => {
        const d = Object.fromEntries(choices("container").map((c) => [c.value, c.description]));
        expect(d.default).toMatch(/refused/);
        expect(d.plan).toBe("No tool execution — read-only planning");
    });
});
