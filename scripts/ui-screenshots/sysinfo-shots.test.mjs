// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The sysinfo suite's list of plots: what it can check without a running instance. Capturing needs one.

import { describe, expect, it } from "vitest";
import { PLOTS, shots } from "./sysinfo-shots.mjs";

/** The plot types the pane saves for its simple views (frontend/app/view/sysinfo/sysinfo-types.ts, SIMPLE_VIEWS). */
const SIMPLE_TYPES = ["CPU", "All CPU", "Mem", "Net", "Net (Sent/Recv)", "Disk I/O", "Disk I/O (R/W)", "CPU + Mem", "CPU + Mem + Net"];
/** The Advanced views' areas (sysinfo-overview-data.ts, FOCUS_CHOICES). */
const AREAS = ["cpu", "mem", "disk", "net", "everything"];

describe("the sysinfo suite", () => {
    it("has one shot per plot, named as the docs image, with no id twice", () => {
        expect(shots.map((s) => s.id)).toEqual(PLOTS.map((p) => p.id));
        expect(new Set(PLOTS.map((p) => p.id)).size).toBe(PLOTS.length);
        for (const p of PLOTS) expect(p.id).toMatch(/^sysinfo-[a-z-]+$/);
    });

    it("covers every simple view and every Advanced area once", () => {
        const simple = PLOTS.filter((p) => !p.focus).map((p) => p.type);
        expect([...simple].sort()).toEqual([...SIMPLE_TYPES].sort());
        const advanced = PLOTS.filter((p) => p.focus).map((p) => p.focus);
        expect([...advanced].sort()).toEqual([...AREAS].sort());
    });

    it("saves a simple view with its metrics and an Advanced one with its area", () => {
        for (const p of PLOTS) {
            if (p.focus) expect(p.type).toBe("Overview");
            else expect(p.metrics === "cores" || (Array.isArray(p.metrics) && p.metrics.length > 0)).toBe(true);
        }
    });

    it("is captured one shot at a time, in one size, each in a tab of its own", () => {
        for (const s of shots) {
            expect(s.sizes).toBeUndefined();
            expect(typeof s.prep).toBe("function");
            expect(typeof s.cleanup).toBe("function");
            expect(typeof s.selector).toBe("function");
        }
    });
});
