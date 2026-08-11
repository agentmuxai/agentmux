// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, test } from "vitest";
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
