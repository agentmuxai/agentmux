// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { effectiveModel, effectiveRuntime, effortFromFlags, permissionModeFromFlags, effortApplies, effortNotUsedReason, modelFromFlags, modelTakesEffort } from "./runtime-capabilities";

describe("modelTakesEffort", () => {
    it("Haiku does not, by alias or by concrete id, in any case", () => {
        for (const m of ["haiku", "Haiku", "claude-haiku-4-5", "claude-haiku-4-5-20251001"]) {
            expect(modelTakesEffort(m), m).toBe(false);
        }
    });
    it("every other model does", () => {
        for (const m of ["sonnet", "opus", "claude-sonnet-5-5", "claude-opus-5-5", "claude-fable-5-1", "gpt-5.5"]) {
            expect(modelTakesEffort(m), m).toBe(true);
        }
    });
});

describe("effortApplies", () => {
    it("only Claude takes --effort, and not on Haiku", () => {
        expect(effortApplies("claude", "sonnet")).toBe(true);
        expect(effortApplies(undefined, "sonnet")).toBe(true);
        expect(effortApplies("claude", "claude-haiku-4-5-20251001")).toBe(false);
        for (const p of ["codex", "gemini", "kimi", "qwen", "antigravity", "copilot"]) {
            expect(effortApplies(p, "sonnet"), p).toBe(false);
        }
    });
    it("says why when it does not", () => {
        expect(effortNotUsedReason("claude", "sonnet")).toBeNull();
        expect(effortNotUsedReason("claude", "haiku")).toMatch(/Haiku/);
        expect(effortNotUsedReason("codex", "gpt-5.5")).toMatch(/codex has no effort/);
    });
});

describe("modelFromFlags / effectiveModel", () => {
    it("reads the model a flag list selects, the last one winning", () => {
        expect(modelFromFlags(["--model", "opus"])).toBe("opus");
        expect(modelFromFlags(["-m", "gpt-5.5"])).toBe("gpt-5.5");
        expect(modelFromFlags(["--model=haiku"])).toBe("haiku");
        expect(modelFromFlags(["--model", "opus", "--x", "--model", "haiku"])).toBe("haiku");
        expect(modelFromFlags(["--add-dir", "/tmp"])).toBeUndefined();
        expect(modelFromFlags(["--model"])).toBeUndefined();
    });
    it("the definition's own --model beats the runtime's, as it does on the command line", () => {
        expect(effectiveModel("sonnet", "--model claude-haiku-4-5-20251001")).toBe("claude-haiku-4-5-20251001");
        expect(effectiveModel("sonnet", "--add-dir /tmp")).toBe("sonnet");
        expect(effectiveModel("sonnet", "")).toBe("sonnet");
        expect(effectiveModel("sonnet", undefined)).toBe("sonnet");
    });
});

describe("effectiveRuntime", () => {
    const base = { permissionMode: "bypass", model: "sonnet", effort: "high" };
    it("the definition's own --model and --effort win over the stored selection", () => {
        expect(effectiveRuntime(base, "--model opus --effort max")).toEqual({ ...base, model: "opus", effort: "max" });
        expect(effectiveRuntime(base, "--effort=low")).toEqual({ ...base, effort: "low" });
    });
    it("the selection stands when the flags say nothing about it", () => {
        expect(effectiveRuntime(base, "--add-dir /tmp")).toEqual(base);
        expect(effectiveRuntime(base, undefined)).toEqual(base);
    });
    it("reads the last of a repeated effort", () => {
        expect(effortFromFlags(["--effort", "low", "--effort", "max"])).toBe("max");
        expect(effortFromFlags(["--effort"])).toBeUndefined();
    });
});

describe("permissionModeFromFlags / effectiveRuntime's mode", () => {
    it("reads the mode a flag list selects; the last wins; bypass flags read as bypass", () => {
        expect(permissionModeFromFlags(["--permission-mode", "plan"])).toBe("plan");
        expect(permissionModeFromFlags(["--permission-mode=acceptEdits"])).toBe("acceptEdits");
        expect(permissionModeFromFlags(["--dangerously-skip-permissions"])).toBe("bypass");
        expect(permissionModeFromFlags(["--yolo"])).toBe("bypass");
        expect(permissionModeFromFlags(["--permission-mode", "plan", "--dangerously-skip-permissions"])).toBe("bypass");
        expect(permissionModeFromFlags(["--dangerously-skip-permissions", "--permission-mode", "plan"])).toBe("plan");
        expect(permissionModeFromFlags(["--add-dir", "/x"])).toBeUndefined();
        expect(permissionModeFromFlags(["--permission-mode"])).toBeUndefined();
    });
    it("the definition's own mode wins over the stored selection (ReAgent P2 on #4161)", () => {
        const base = { permissionMode: "bypass", model: "sonnet", effort: "high" };
        expect(effectiveRuntime(base, "--permission-mode plan").permissionMode).toBe("plan");
        expect(effectiveRuntime(base, "--add-dir /tmp").permissionMode).toBe("bypass");
    });
});
