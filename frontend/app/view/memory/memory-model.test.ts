// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, test, vi } from "vitest";
import {
    draftFromMemory,
    draftToWire,
    duplicateProviderKeys,
    emptyDraft,
    parseInstructionsByProvider,
    providerKeyProblem,
    sanitizeProviderKey,
    serializeInstructionsByProvider,
} from "./memory-model";

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

// provider/model — ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7.
// Previously forced empty on every save (SPEC_MEMORY_IDENTITY_ARCH §4.1a:
// "presets are provider-agnostic"). That decision is reversed: an ABF now
// carries its own provider + model, readonly once set (backend-enforced in
// bundle.upsert), so an exported ABF is self-describing about what it needs
// to run. Same class of round-trip concern as instructions_by_provider
// above — must not be silently dropped on an edit to an unrelated field.
describe("provider/model round-trip", () => {
    test("draftFromMemory preserves populated values", () => {
        const draft = draftFromMemory({
            id: "b1",
            name: "Test",
            provider: "claude",
            model: "anthropic",
            created_at: 0,
            updated_at: 0,
        } as Memory);
        expect(draft.provider).toBe("claude");
        expect(draft.model).toBe("anthropic");
    });

    test("draftFromMemory falls back to empty strings for an absent value", () => {
        const draft = draftFromMemory({
            id: "b1",
            name: "Test",
            created_at: 0,
            updated_at: 0,
        } as Memory);
        expect(draft.provider).toBe("");
        expect(draft.model).toBe("");
    });

    test("emptyDraft starts with empty provider/model (unset until creation)", () => {
        expect(emptyDraft().provider).toBe("");
        expect(emptyDraft().model).toBe("");
    });

    test("draftToWire sends the draft's provider/model through unchanged", () => {
        const draft = { ...emptyDraft(), name: "Test", provider: "codex", model: "openai" };
        const wire = draftToWire(draft);
        expect(wire.provider).toBe("codex");
        expect(wire.model).toBe("openai");
    });

    test("full round trip: an edit to an unrelated field does not drop provider/model", () => {
        const stored = {
            id: "b1",
            name: "Original name",
            provider: "claude",
            model: "anthropic",
            created_at: 0,
            updated_at: 0,
        } as Memory;
        const draft = draftFromMemory(stored);
        draft.description = "Added a description"; // simulates editing an unrelated field
        const wire = draftToWire(draft);
        expect(wire.provider).toBe("claude");
        expect(wire.model).toBe("anthropic");
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

// SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md — same hub pattern
// bundle-mcp-model.test.ts / global-brain-model.test.ts use.
const wpsHub = vi.hoisted(() => ({ handlers: new Map<string, (e: unknown) => void>() }));
vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        wpsHub.handlers.set(sub.eventType, sub.handler);
        return () => wpsHub.handlers.delete(sub.eventType);
    }),
}));

describe("validateDraft", () => {
    beforeEach(() => {
        listMemoriesMock.mockClear();
        validateBundleMock.mockClear();
        wpsHub.handlers.clear();
    });

    // SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md
    test("subscribes to memories:changed and refreshes on it; unsubscribes on dispose", async () => {
        const { MemoryViewModel } = await import("./memory-model");
        const model = new MemoryViewModel();
        await Promise.resolve();
        listMemoriesMock.mockClear();

        wpsHub.handlers.get("memories:changed")?.({});
        await Promise.resolve();
        expect(listMemoriesMock).toHaveBeenCalledTimes(1);

        model.dispose();
        expect(wpsHub.handlers.has("memories:changed")).toBe(false);
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

// -- ABF v0.2 §2.2 per-provider instructions: authoring-UI model layer --
//
// The authoring UI edits a parsed list, but the draft still carries the raw
// JSON string, because that string is what round-trips (see the suite above,
// which exists because reagent P1 on #2523 caught an edit silently wiping an
// imported bundle's variants). These functions are the seam between the two,
// so the wipe-safety property has to hold HERE or the UI reintroduces the bug.
describe("instructions_by_provider authoring model", () => {
    it("parses an object into sorted entries", () => {
        const r = parseInstructionsByProvider('{"codex":"C","claude":"A"}');
        expect(r.malformed).toBe(false);
        expect(r.entries).toEqual([
            { provider: "claude", content: "A" },
            { provider: "codex", content: "C" },
        ]);
    });

    it("treats empty/missing as an empty, editable set (not malformed)", () => {
        for (const raw of ["", "   ", "{}"]) {
            const r = parseInstructionsByProvider(raw);
            expect(r.malformed).toBe(false);
            expect(r.entries).toEqual([]);
        }
    });

    // THE wipe-safety property. Malformed input must be reported as such and
    // must NOT yield an empty entry list that the UI would then serialize back
    // over the top of, destroying variants it merely failed to parse.
    it("flags malformed JSON as malformed rather than as empty", () => {
        for (const raw of ["not json", "[1,2]", '"a string"', "null", "42"]) {
            const r = parseInstructionsByProvider(raw);
            expect(r.malformed).toBe(true);
            expect(r.entries).toEqual([]);
        }
    });

    it("ignores non-string values rather than coercing them", () => {
        const r = parseInstructionsByProvider('{"claude":"ok","codex":123}');
        expect(r.malformed).toBe(true);
        expect(r.entries).toEqual([]);
    });

    it("serializes entries back to a JSON object", () => {
        expect(
            serializeInstructionsByProvider([
                { provider: "claude", content: "A" },
                { provider: "codex", content: "C" },
            ]),
        ).toBe('{"claude":"A","codex":"C"}');
    });

    it("drops rows with a blank provider key on serialize", () => {
        expect(
            serializeInstructionsByProvider([
                { provider: "  ", content: "orphaned" },
                { provider: "claude", content: "A" },
            ]),
        ).toBe('{"claude":"A"}');
    });

    it("trims the provider key but never the content", () => {
        expect(
            serializeInstructionsByProvider([{ provider: " claude ", content: "  padded  " }]),
        ).toBe(JSON.stringify({ claude: "  padded  " }));
    });

    it("round-trips parse -> serialize without altering content", () => {
        const raw = JSON.stringify({ claude: ["line1", "line2"].join("\n"), codex: "  padded  " });
        const { entries } = parseInstructionsByProvider(raw);
        expect(serializeInstructionsByProvider(entries)).toBe(raw);
    });

    // Authoring-time mirror of the export-side rule. These assertions are
    // pinned to sanitize_context_relative_path (bundle_export.rs) EXACTLY —
    // reagent P1 on #3063 caught the first version inventing stricter rules
    // than the backend has, so it warned "It will not be saved" about keys
    // that in fact export fine. A mirror that disagrees with what it mirrors
    // is worse than no mirror.
    it("rejects only the keys the backend actually rejects", () => {
        // empty / no usable segment
        expect(providerKeyProblem("")).not.toBeNull();
        expect(providerKeyProblem("   ")).not.toBeNull();
        expect(providerKeyProblem(".")).not.toBeNull();
        expect(providerKeyProblem("./.")).not.toBeNull();
        expect(providerKeyProblem("/")).not.toBeNull();
        // leading separator, colon, traversal
        expect(providerKeyProblem("/abs")).not.toBeNull();
        expect(providerKeyProblem("C:/x")).not.toBeNull();
        expect(providerKeyProblem("a:b")).not.toBeNull();
        expect(providerKeyProblem("..")).not.toBeNull();
        expect(providerKeyProblem("a/../b")).not.toBeNull();
    });

    // The backend normalizes "\" to "/" and splits — nested segments are legal
    // and export to instructions/a/b/AGENTS.md. Warning about them was the
    // actual defect in #3063's first cut.
    it("accepts keys the backend accepts, including nested and odd ones", () => {
        for (const ok of ["claude", "codex", "my-harness", "my_harness2", "a/b"]) {
            expect(providerKeyProblem(ok)).toBeNull();
        }
        // Real backslash, via String.raw. The previous version of this test
        // wrote "a\b", which JS reads as a BACKSPACE control character, so the
        // separator case was never exercised and passed only by accident
        // (reagent P2, #3063).
        expect(providerKeyProblem(String.raw`a\b`)).toBeNull();
        // Control characters are not rejected by the backend, so we must not
        // claim they are.
        expect(providerKeyProblem("a\bb")).toBeNull();
    });

    it("sanitizes to the path export will actually write", () => {
        expect(sanitizeProviderKey("claude")).toBe("claude");
        expect(sanitizeProviderKey(String.raw`a\b`)).toBe("a/b");
        expect(sanitizeProviderKey("./a//b")).toBe("a/b");
        expect(sanitizeProviderKey("..")).toBeNull();
    });

    // Export keys its collision set on the SANITIZED path, so two keys that
    // differ as strings but normalize identically are a real collision — one
    // of them is silently dropped from the .abf.
    it("flags duplicates that collide only after sanitizing", () => {
        expect(
            duplicateProviderKeys([
                { provider: "a/b", content: "A" },
                { provider: String.raw`a\b`, content: "B" },
            ]),
        ).toEqual([String.raw`a\b`]);
        expect(
            duplicateProviderKeys([
                { provider: "claude", content: "A" },
                { provider: "claude", content: "B" },
            ]),
        ).toEqual(["claude"]);
        expect(duplicateProviderKeys([{ provider: "claude", content: "A" }])).toEqual([]);
    });

    it("does not report backend-rejected keys as duplicates", () => {
        // They are already flagged by providerKeyProblem; double-warning about
        // a key that never reaches export is noise.
        expect(
            duplicateProviderKeys([
                { provider: "..", content: "A" },
                { provider: "..", content: "B" },
            ]),
        ).toEqual([]);
    });
});
