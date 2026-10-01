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
