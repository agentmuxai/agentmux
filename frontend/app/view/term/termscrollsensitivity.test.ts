// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    DEFAULT_TERM_SCROLL_SENSITIVITY,
    MAX_TERM_SCROLL_SENSITIVITY,
    MIN_TERM_SCROLL_SENSITIVITY,
    resolveTermScrollSensitivity,
} from "./termscrollsensitivity";

describe("resolveTermScrollSensitivity", () => {
    it("falls back to the default when nothing is configured", () => {
        expect(resolveTermScrollSensitivity(undefined)).toBe(DEFAULT_TERM_SCROLL_SENSITIVITY);
        expect(resolveTermScrollSensitivity(null)).toBe(DEFAULT_TERM_SCROLL_SENSITIVITY);
    });

    it("uses the configured value when present and in range", () => {
        expect(resolveTermScrollSensitivity(3)).toBe(3);
        expect(resolveTermScrollSensitivity(0.5)).toBe(0.5);
    });

    it("clamps to the schema maximum instead of rejecting an over-range value", () => {
        // settings.json documents 0.1-10 (schema/settings.json's
        // term:scrollsensitivity), but the setting can be hand-edited past
        // that via the raw settings.json escape hatch. Clamping (not
        // falling back to the default) honors the user's intent to go fast
        // rather than silently reverting to "normal".
        expect(resolveTermScrollSensitivity(50)).toBe(MAX_TERM_SCROLL_SENSITIVITY);
        expect(resolveTermScrollSensitivity(10)).toBe(10);
    });

    it("clamps a positive but below-minimum value to the schema minimum", () => {
        expect(resolveTermScrollSensitivity(0.01)).toBe(MIN_TERM_SCROLL_SENSITIVITY);
    });

    it("ignores zero, negative, and non-numeric values rather than propagating them", () => {
        // A malformed settings file must not reach xterm as NaN/0/negative —
        // each of these falls through to the default, same guard termwrap.ts
        // applied inline before this was centralized.
        expect(resolveTermScrollSensitivity(0)).toBe(DEFAULT_TERM_SCROLL_SENSITIVITY);
        expect(resolveTermScrollSensitivity(-2)).toBe(DEFAULT_TERM_SCROLL_SENSITIVITY);
        expect(resolveTermScrollSensitivity("fast" as unknown as number)).toBe(DEFAULT_TERM_SCROLL_SENSITIVITY);
        expect(resolveTermScrollSensitivity(Number.NaN)).toBe(DEFAULT_TERM_SCROLL_SENSITIVITY);
        expect(resolveTermScrollSensitivity(Number.POSITIVE_INFINITY)).toBe(DEFAULT_TERM_SCROLL_SENSITIVITY);
    });

    /**
     * The regression this module exists to prevent, mirroring
     * termscrollback.ts's own closing test: a value the user actually
     * configured must reach xterm.js, not collapse back to the default —
     * verified once as a shared resolver rather than trusted separately in
     * termwrap.ts, termViewModel.ts, and AgentShellSubblock.tsx.
     */
    it("honors a lowered setting instead of collapsing to the default", () => {
        const resolved = resolveTermScrollSensitivity(0.2);
        expect(resolved).toBe(0.2);
        expect(resolved).not.toBe(DEFAULT_TERM_SCROLL_SENSITIVITY);
    });
});
