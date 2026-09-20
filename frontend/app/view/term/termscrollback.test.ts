// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    DEFAULT_TERM_SCROLLBACK,
    MAX_TERM_SCROLLBACK,
    resolveTermScrollback,
} from "./termscrollback";

describe("resolveTermScrollback", () => {
    it("falls back to the default when nothing is configured", () => {
        expect(resolveTermScrollback(undefined)).toBe(DEFAULT_TERM_SCROLLBACK);
        expect(resolveTermScrollback({})).toBe(DEFAULT_TERM_SCROLLBACK);
        expect(resolveTermScrollback(null, null)).toBe(DEFAULT_TERM_SCROLLBACK);
    });

    it("uses the term:scrollback setting when present", () => {
        expect(resolveTermScrollback({ "term:scrollback": 10000 })).toBe(10000);
    });

    it("lets a per-block override win over the setting", () => {
        expect(resolveTermScrollback({ "term:scrollback": 10000 }, { "term:scrollback": 25000 })).toBe(25000);
    });

    it("clamps to the maximum", () => {
        expect(resolveTermScrollback({ "term:scrollback": 999999 })).toBe(MAX_TERM_SCROLLBACK);
    });

    it("floors fractional values", () => {
        expect(resolveTermScrollback({ "term:scrollback": 4200.9 })).toBe(4200);
    });

    it("ignores zero, negative, and non-numeric values rather than propagating them", () => {
        // A malformed settings file must not reach xterm as NaN/0 — each of
        // these falls through to the next source of truth.
        expect(resolveTermScrollback({ "term:scrollback": 0 })).toBe(DEFAULT_TERM_SCROLLBACK);
        expect(resolveTermScrollback({ "term:scrollback": -5 })).toBe(DEFAULT_TERM_SCROLLBACK);
        expect(resolveTermScrollback({ "term:scrollback": "lots" as unknown as number })).toBe(
            DEFAULT_TERM_SCROLLBACK
        );
        expect(resolveTermScrollback({ "term:scrollback": Number.NaN })).toBe(DEFAULT_TERM_SCROLLBACK);
    });

    it("falls back to the setting when the block override is malformed", () => {
        expect(resolveTermScrollback({ "term:scrollback": 8000 }, { "term:scrollback": 0 })).toBe(8000);
    });

    /**
     * The regression this module exists for: the agent Shell drawer used to
     * hardcode 2000 and ignore the setting, so raising `term:scrollback` had no
     * effect there and long sessions lost the top of their history. Both
     * surfaces now resolve through this one function, so a configured value
     * must never silently collapse back to the default.
     */
    it("honors a raised setting instead of collapsing to the hardcoded drawer default", () => {
        const resolved = resolveTermScrollback({ "term:scrollback": 50000 });
        expect(resolved).toBe(50000);
        expect(resolved).not.toBe(DEFAULT_TERM_SCROLLBACK);
    });
});
