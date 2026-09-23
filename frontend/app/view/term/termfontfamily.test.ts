// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { DEFAULT_TERM_FONT_FAMILY, resolveTermFontFamily } from "./termfontfamily";

describe("resolveTermFontFamily", () => {
    it("falls back to the default when nothing is configured", () => {
        expect(resolveTermFontFamily(undefined)).toBe(DEFAULT_TERM_FONT_FAMILY);
        expect(resolveTermFontFamily({})).toBe(DEFAULT_TERM_FONT_FAMILY);
        expect(resolveTermFontFamily(undefined, undefined)).toBe(DEFAULT_TERM_FONT_FAMILY);
    });

    it("uses the term:fontfamily setting when present", () => {
        expect(resolveTermFontFamily({ "term:fontfamily": "Fira Code" })).toBe("Fira Code");
    });

    // Precedence is settings-before-connection, matching term.tsx's own
    // pre-existing `ts?.["term:fontfamily"] ?? connFontFamily ?? "Hack"` —
    // deliberately preserved as-is, not reconciled with term:fontsize's
    // opposite (meta-before-connection-before-settings) precedence. See
    // SPEC_SETTINGS_LIVE_COMMIT_AND_TERMINAL_APPLY_GAPS_2026_09_22.md §6.1.
    it("lets the global setting win over a connection-level font family", () => {
        expect(resolveTermFontFamily({ "term:fontfamily": "Fira Code" }, "Menlo")).toBe("Fira Code");
    });

    it("falls back to the connection font family when no global setting is present", () => {
        expect(resolveTermFontFamily({}, "Menlo")).toBe("Menlo");
        expect(resolveTermFontFamily(undefined, "Menlo")).toBe("Menlo");
    });

    it("falls back to the default when neither the setting nor a connection value is present", () => {
        expect(resolveTermFontFamily({}, undefined)).toBe(DEFAULT_TERM_FONT_FAMILY);
        expect(resolveTermFontFamily({}, null)).toBe(DEFAULT_TERM_FONT_FAMILY);
    });

    it("ignores a non-string setting value rather than propagating it", () => {
        expect(resolveTermFontFamily({ "term:fontfamily": 42 as unknown as string }, "Menlo")).toBe("Menlo");
    });
});
