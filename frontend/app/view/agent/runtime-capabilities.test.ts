// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { PermissionMode } from "./types";
import { effectiveModel, effectiveRuntime, effortFromFlags, permissionModeFromFlags, permissionModeText, effortApplies, effortNotUsedReason, modelFromFlags, modelTakesEffort } from "./runtime-capabilities";

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
    it("the definition's own mode wins over the stored selection (muxreview P2 on #4161)", () => {
        const base = { permissionMode: "bypass", model: "sonnet", effort: "high" };
        expect(effectiveRuntime(base, "--permission-mode plan").permissionMode).toBe("plan");
        expect(effectiveRuntime(base, "--add-dir /tmp").permissionMode).toBe("bypass");
    });
});

describe("permissionModeText", () => {
    it("makes no claim for a provider whose CLI was not observed", () => {
        for (const id of ["gemini", "kimi", "qwen", "codex", ""]) {
            for (const persistent of [true, false]) {
                for (const m of ["bypass", "default", "acceptEdits", "auto", "plan"] as const) {
                    const t = permissionModeText(m, persistent, id);
                    expect(t.note).toBeUndefined();
                    expect(t.label).not.toMatch(/read-only|prompt all|refused/i);
                }
            }
        }
    });

    const MODES: PermissionMode[] = ["bypass", "auto", "acceptEdits", "plan", "default"];

    it("where nothing can be asked (a container's one-shot run), the CLI refuses what the mode does not allow", () => {
        // Observed on CLI 2.1.285 without a permission prompt tool: Default refuses an
        // unapproved write or command; Plan refuses writes and blocks commands.
        expect(permissionModeText("bypass", false).label).toBe("Bypass (no prompts)");
        expect(permissionModeText("plan", false).label).toBe("Plan (read-only)");
        expect(permissionModeText("default", false).note).toMatch(/refused/);
        expect(permissionModeText("default", false).label).not.toMatch(/prompt all/i);
        expect(permissionModeText("acceptEdits", false).note).toMatch(/edits are allowed.*refused/);
        expect(permissionModeText("auto", false).label).toBe("Auto (AI classifier)");
    });

    it("where the server answers every ask, no mode promises prompting - or read-only-ness", () => {
        for (const m of MODES) {
            const t = permissionModeText(m, true);
            expect(t.label, m).not.toMatch(/prompt all|AI classifier|read-only/i);
        }
        expect(permissionModeText("default", true).note).toMatch(/allowed automatically/);
        expect(permissionModeText("acceptEdits", true).note).toMatch(/allowed automatically/);
    });

    it("Plan on a persistent agent does NOT stop edits: writes are asked about and allowed (observed)", () => {
        // The first wording said "read-only while planning". Running the CLI showed a write in
        // plan mode is ASKED about, not refused, and an "allow" lets it happen.
        const note = permissionModeText("plan", true).note ?? "";
        expect(note).toMatch(/does NOT stop edits/);
        expect(note).not.toMatch(/read-only/i);
        expect(note).toMatch(/plan is approved automatically/);
    });

    it("Bypass says nothing false either way, so it needs no note", () => {
        expect(permissionModeText("bypass", true)).toEqual({ label: "Bypass (no prompts)" });
        expect(permissionModeText("bypass", false)).toEqual({ label: "Bypass (no prompts)" });
    });

    // `Record<PermissionMode, true>` makes this a COMPILE-TIME list: adding a mode to
    // the PermissionMode union without adding it here is a type error, so a new
    // mode cannot ship without someone deciding what its menu text says.
    const EVERY_MODE: Record<PermissionMode, true> = { bypass: true, auto: true, acceptEdits: true, plan: true, default: true };

    it("every permission mode has wording for both kinds of controller", () => {
        for (const mode of Object.keys(EVERY_MODE) as PermissionMode[]) {
            for (const autoAnswers of [true, false]) {
                const t = permissionModeText(mode, autoAnswers);
                expect(t.label.length, `${mode}/${autoAnswers}`).toBeGreaterThan(0);
                // Where a note exists: every persistent mode but Bypass (the server answers its
                // asks), and a one-shot run's Default and Accept Edits (they refuse, which the
                // label alone does not say).
                const needsNote = autoAnswers ? mode !== "bypass" : mode === "default" || mode === "acceptEdits";
                expect(Boolean(t.note), `${mode}/${autoAnswers}`).toBe(needsNote);
            }
        }
    });
});
