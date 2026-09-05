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
 */

import { describe, expect, it } from "vitest";

import { getProvider, setProviderModels } from "./model-overlay";

/** Shape the backend's `providers.models` RPC delivers. */
const apiModels = (...ids: Array<[string, string]>) =>
    ids.map(([value, label]) => ({ value, label }));

const claudeModels = () => getProvider("claude")!.models;
const row = (value: string) => claudeModels().find((m) => m.value === value);

describe("setProviderModels", () => {
    it("refreshes an alias row's label without touching its value", () => {
        setProviderModels("claude", apiModels(["claude-opus-6", "Claude Opus 6"]));
        const opus = claudeModels().find((m) => m.label === "Opus 6");
        expect(opus).toBeDefined();
        // The alias must survive — the CLI resolves it to whatever is current.
        expect(opus!.value).toBe("opus");
    });

    it("refreshes a concrete row's value as well as its label", () => {
        setProviderModels("claude", apiModels(["claude-fable-6", "Claude Fable 6"]));
        const fable = claudeModels().find((m) => m.label === "Fable 6");
        expect(fable).toBeDefined();
        // Value moved with the label — no advertise-one/select-another gap.
        expect(fable!.value).toBe("claude-fable-6");
    });

    it("picks the newest family member regardless of API ordering", () => {
        setProviderModels(
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
        setProviderModels(
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
        setProviderModels("claude", apiModels(["claude-mythos-5-1", "Claude Mythos 5.1"]));
        const mythos = claudeModels().find((m) => m.value === "claude-mythos-5-1");
        expect(mythos).toBeDefined();
        expect(mythos!.label).toBe("Mythos 5.1");
    });

    it("leaves the static catalog untouched when the API returns nothing", () => {
        const before = claudeModels().map((m) => m.value);
        setProviderModels("claude", []);
        expect(claudeModels().map((m) => m.value)).toEqual(before);
    });

    it("preserves the default marker through a refresh", () => {
        setProviderModels("claude", apiModels(["claude-sonnet-6", "Claude Sonnet 6"]));
        const dflt = claudeModels().find((m) => m.default);
        expect(dflt?.value).toBe("sonnet");
    });
});

describe("curated catalog", () => {
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
});
