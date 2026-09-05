// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The live-catalog overlay's contract.
 *
 * The distinction under test: an ALIAS row ("opus") is resolved by the CLI at
 * call time, so the overlay must refresh only its label and never its value —
 * self-resolution is the point. A CONCRETE row ("claude-fable-5-1") does not
 * self-resolve, so its value must be refreshed alongside its label.
 *
 * Getting this wrong is not cosmetic: refreshing only the label on a concrete
 * row produces a picker entry that displays "Fable 5.1" while still passing
 * `claude-fable-5` to `--model` — advertising one model and selecting an older
 * one, with nothing in the UI indicating the mismatch.
 *
 * `setProviderModels` writes to a module-level signal, so every test re-imports
 * the module through `vi.resetModules()` to get a clean catalog. Without that,
 * the "curated catalog" assertions below would run against state left by
 * earlier tests rather than the curated defaults they mean to check — passing
 * or failing depending on execution order. reagent P2, PR #2990.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

type Overlay = typeof import("./model-overlay");
let overlay: Overlay;

beforeEach(async () => {
    vi.resetModules();
    overlay = await import("./model-overlay");
});

/** Shape the backend's `providers.models` RPC delivers. */
const apiModels = (...ids: Array<[string, string]>) =>
    ids.map(([value, label]) => ({ value, label }));

const claudeModels = () => overlay.getProvider("claude")!.models;

describe("setProviderModels", () => {
    it("refreshes an alias row's label without touching its value", () => {
        overlay.setProviderModels("claude", apiModels(["claude-opus-6", "Claude Opus 6"]));
        const opus = claudeModels().find((m) => m.label === "Opus 6");
        expect(opus).toBeDefined();
        // The alias must survive — the CLI resolves it to whatever is current.
        expect(opus!.value).toBe("opus");
    });

    it("refreshes a concrete row's value as well as its label", () => {
        overlay.setProviderModels("claude", apiModels(["claude-fable-6", "Claude Fable 6"]));
        const fable = claudeModels().find((m) => m.label === "Fable 6");
        expect(fable).toBeDefined();
        // Value moved with the label — no advertise-one/select-another gap.
        expect(fable!.value).toBe("claude-fable-6");
    });

    it("picks the newest family member regardless of API ordering", () => {
        overlay.setProviderModels(
            "claude",
            apiModels(
                ["claude-fable-5-1", "Claude Fable 5.1"],
                ["claude-fable-5", "Claude Fable 5"],
            ),
        );
        const fable = claudeModels().find((m) => m.label.startsWith("Fable"));
        expect(fable!.value).toBe("claude-fable-5-1");
        expect(fable!.label).toBe("Fable 5.1");
    });

    it("does not emit a duplicate row for a family it already covers", () => {
        overlay.setProviderModels(
            "claude",
            apiModels(
                ["claude-fable-5", "Claude Fable 5"],
                ["claude-fable-5-1", "Claude Fable 5.1"],
            ),
        );
        const fableRows = claudeModels().filter((m) => m.label.startsWith("Fable"));
        expect(fableRows).toHaveLength(1);
    });

    it("appends an unseen family rather than dropping it", () => {
        overlay.setProviderModels("claude", apiModels(["claude-mythos-5-1", "Claude Mythos 5.1"]));
        const mythos = claudeModels().find((m) => m.value === "claude-mythos-5-1");
        expect(mythos).toBeDefined();
        expect(mythos!.label).toBe("Mythos 5.1");
    });

    it("leaves the static catalog untouched when the API returns nothing", () => {
        const before = claudeModels().map((m) => m.value);
        overlay.setProviderModels("claude", []);
        expect(claudeModels().map((m) => m.value)).toEqual(before);
    });

    it("preserves the default marker through a refresh", () => {
        overlay.setProviderModels("claude", apiModels(["claude-sonnet-6", "Claude Sonnet 6"]));
        const dflt = claudeModels().find((m) => m.default);
        expect(dflt?.value).toBe("sonnet");
    });
});

describe("curated catalog", () => {
    // These read the UNOVERLAID catalog — hence the per-test module reset.
    it("keeps version numbers out of descriptions so they cannot contradict labels", () => {
        // A description naming a version goes stale the moment the label is
        // refreshed — the "Fable 5.1 over Claude Fable 5" report.
        for (const m of claudeModels()) {
            expect(m.description ?? "").not.toMatch(/\d/);
        }
    });

    it("pins the fable row to a concrete id, not a bare family name", () => {
        // There is no documented `fable` CLI alias, so the row must carry a
        // full model name for `--model` to resolve.
        const fable = claudeModels().find((m) => m.label.startsWith("Fable"));
        expect(fable!.value).toMatch(/^claude-fable-/);
    });

    it("ships the current fable id, not the superseded one", () => {
        const fable = claudeModels().find((m) => m.label.startsWith("Fable"));
        expect(fable!.value).toBe("claude-fable-5-1");
    });
});

describe("familyKey (exported for selection migration)", () => {
    it("groups versions of the same family so a stale selection can be migrated", () => {
        // AgentCreateFromTemplateModal and AgentRuntimeDropup both use this to
        // move a selection when the overlay replaces a concrete value.
        expect(overlay.familyKey("claude-fable-5")).toBe(overlay.familyKey("claude-fable-5-1"));
        expect(overlay.familyKey("claude-fable-5")).not.toBe(overlay.familyKey("claude-opus-5"));
        expect(overlay.familyKey("opus")).toBe("opus");
    });
});
