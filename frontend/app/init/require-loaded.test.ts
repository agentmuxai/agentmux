// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A core startup object coming back null used to surface as
 * `TypeError: Cannot read properties of null (reading 'layoutstate')` thrown
 * from inside init — which init swallowed, leaving a completely blank window
 * and one console line. These pin the two properties that fix relies on: the
 * failure names what was missing, and it is typed so init can re-throw it
 * rather than swallowing it with the tolerable late failures.
 */

import { describe, expect, it } from "vitest";

import { MuxInitFatalError, requireLoaded } from "./require-loaded";

describe("requireLoaded", () => {
    it("passes a loaded object straight through", () => {
        const tab = { layoutstate: "layout-1" };
        expect(requireLoaded(tab, "tab", "tab-1")).toBe(tab);
    });

    it("does not reject falsy-but-present values", () => {
        // A legitimately empty string / 0 / false must not be mistaken for a
        // failed load — only null and undefined mean "did not load".
        expect(requireLoaded("", "tab", "tab-1")).toBe("");
        expect(requireLoaded(0, "tab", "tab-1")).toBe(0);
        expect(requireLoaded(false, "tab", "tab-1")).toBe(false);
    });

    it("throws a typed error naming the object and id, for null and undefined", () => {
        for (const missing of [null, undefined]) {
            expect(() => requireLoaded(missing, "tab", "tab-abc")).toThrow(MuxInitFatalError);
            try {
                requireLoaded(missing, "tab", "tab-abc");
                throw new Error("should have thrown");
            } catch (e) {
                // Naming both is the whole point: the old TypeError said only
                // which *field* was read, never which object or which id.
                expect(String((e as Error).message)).toContain("tab");
                expect(String((e as Error).message)).toContain("tab-abc");
            }
        }
    });

    it("is distinguishable from an ordinary Error so init can re-throw selectively", () => {
        // init tolerates late non-fatal failures and swallows them; it must be
        // able to tell those apart from this one by type alone.
        const fatal: unknown = new MuxInitFatalError("boom");
        const ordinary: unknown = new TypeError("boom");
        expect(fatal instanceof MuxInitFatalError).toBe(true);
        expect(ordinary instanceof MuxInitFatalError).toBe(false);
        expect((fatal as Error).name).toBe("MuxInitFatalError");
    });
});
