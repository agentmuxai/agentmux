// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, test, vi } from "vitest";
import { draftFromMemory, draftToWire, emptyDraft } from "./memory-model";

// reagent P1, PR #2523: instructions_by_provider (ABF v0.2 §2.2) was
// previously dropped by the edit round-trip. Since this form has no field
// that edits it yet, the draft must carry the raw JSON through unchanged —
// dropping it would default to "{}" on save, and bundle_memory_upsert's
// ON CONFLICT UPDATE unconditionally overwrites the column, silently
// wiping out any variants an import brought in.
describe("instructions_by_provider round-trip", () => {
    test("draftFromMemory preserves a populated value", () => {
        const draft = draftFromMemory({
            id: "b1",
            name: "Test",
            instructions_by_provider: '{"claude":"Claude-specific."}',
            created_at: 0,
            updated_at: 0,
        } as Memory);
        expect(draft.instructions_by_provider).toBe('{"claude":"Claude-specific."}');
    });

    test("draftFromMemory falls back to {} for an absent/blank value", () => {
        const draft = draftFromMemory({
            id: "b1",
            name: "Test",
            created_at: 0,
            updated_at: 0,
        } as Memory);
        expect(draft.instructions_by_provider).toBe("{}");
    });

    test("draftToWire preserves the draft's value unchanged", () => {
        const draft = { ...emptyDraft(), name: "Test", instructions_by_provider: '{"codex":"Codex-specific."}' };
        const wire = draftToWire(draft);
        expect(wire.instructions_by_provider).toBe('{"codex":"Codex-specific."}');
    });

    test("full round trip: an edit to an unrelated field does not drop provider variants", () => {
        const stored = {
            id: "b1",
            name: "Original name",
            instructions_by_provider: '{"claude":"Keep me."}',
            created_at: 0,
            updated_at: 0,
        } as Memory;
        const draft = draftFromMemory(stored);
        draft.name = "Renamed"; // simulates editing an unrelated field
        const wire = draftToWire(draft);
        expect(wire.instructions_by_provider).toBe('{"claude":"Keep me."}');
    });
});

// validateDraft() (Armory Bundle Format (ABF) UI-alignment pass) — the
// Armory bundle editor's "Validate" button. Mocks RpcApi entirely since
// MemoryViewModel's constructor fires an unawaited ListMemoriesCommand
// refresh(); ValidateBundleCommand is the one under test.
const listMemoriesMock = vi.fn().mockResolvedValue([]);
const validateBundleMock = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListMemoriesCommand: (...args: unknown[]) => listMemoriesMock(...args),
        ValidateBundleCommand: (...args: unknown[]) => validateBundleMock(...args),
    },
}));

describe("validateDraft", () => {
    beforeEach(() => {
        listMemoriesMock.mockClear();
        validateBundleMock.mockClear();
    });

    test("populates validationAtom from the RPC response", async () => {
        const { MemoryViewModel } = await import("./memory-model");
        const report: BundleValidationReport = {
            is_valid: false,
            issues: [{ severity: "error", field: "context_files", message: "bad path" }],
        };
        validateBundleMock.mockResolvedValueOnce(report);

        const model = new MemoryViewModel();
        model.startNew();
        await model.validateDraft();

        expect(model.validationAtom()).toEqual(report);
        expect(model.validatingAtom()).toBe(false);
        expect(validateBundleMock).toHaveBeenCalledTimes(1);
    });

    test("is a no-op with no active draft", async () => {
        const { MemoryViewModel } = await import("./memory-model");
        const model = new MemoryViewModel();
        // No startNew()/startEdit() — draftAtom() is null.
        await model.validateDraft();
        expect(validateBundleMock).not.toHaveBeenCalled();
        expect(model.validationAtom()).toBeNull();
    });

    test("a stale response is ignored if the draft changed before the RPC resolved", async () => {
        // reagent P1, PR #2532: Cancel/"+ New Bundle"/another row's Edit
        // are still clickable while validatingAtom() is true, so a report
        // for the OLD draft must not land on whatever draft the user has
        // switched to by the time the RPC resolves. Same race class
        // saveDraft's own identity-equality guard already covers.
        const { MemoryViewModel } = await import("./memory-model");
        let resolveRpc: (report: BundleValidationReport) => void = () => {};
        validateBundleMock.mockReturnValueOnce(
            new Promise<BundleValidationReport>((resolve) => {
                resolveRpc = resolve;
            }),
        );

        const model = new MemoryViewModel();
        model.startNew();
        const inFlight = model.validateDraft();

        // User cancels (or switches to a different draft) while the RPC is
        // still pending.
        model.cancelDraft();

        resolveRpc({ is_valid: true, issues: [] });
        await inFlight;

        expect(model.validationAtom()).toBeNull();
    });

    test("a failed RPC call sets errorAtom instead of validationAtom", async () => {
        const { MemoryViewModel } = await import("./memory-model");
        validateBundleMock.mockRejectedValueOnce(new Error("boom"));

        const model = new MemoryViewModel();
        model.startNew();
        await model.validateDraft();

        expect(model.validationAtom()).toBeNull();
        expect(model.errorAtom()).toContain("boom");
    });

    test("editing a field after validating clears the stale report", async () => {
        const { MemoryViewModel } = await import("./memory-model");
        validateBundleMock.mockResolvedValueOnce({ is_valid: true, issues: [] });

        const model = new MemoryViewModel();
        model.startNew();
        await model.validateDraft();
        expect(model.validationAtom()).not.toBeNull();

        // Simulate memory-manager.tsx's updateDraft() clearing the report
        // on any further edit — a stale "looks good" must not survive a
        // change to the content it described.
        model.setValidation(null);
        expect(model.validationAtom()).toBeNull();
    });
});
