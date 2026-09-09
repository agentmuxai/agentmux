// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { createRedockArming } from "./redock-arming";

// Every test drives the module with explicit timestamps rather than real
// time. `t` is whatever the caller's monotonic clock says; the module never
// reads a clock itself, which is the whole point of extracting it.
const DWELL = 500;
const VEL = 400;

function arming() {
    return createRedockArming({ dwellMs: DWELL, velocityPxPerS: VEL });
}

describe("redock arming — dwell", () => {
    it("arms after the cursor holds the same target for the full dwell", () => {
        const a = arming();
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        expect(a.isArmed()).toBe(false);
        a.sample({ target: "main", x: 0, y: 0, t: 100 });
        expect(a.isArmed()).toBe(false);
        const out = a.sample({ target: "main", x: 0, y: 0, t: 500 });
        expect(out.armed).toBe(true);
        expect(a.isArmed()).toBe(true);
    });

    it("does not arm on a hold shorter than the dwell", () => {
        const a = arming();
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        a.sample({ target: "main", x: 0, y: 0, t: 200 });
        expect(a.isArmed()).toBe(false);
    });

    it("restarts the clock when the target changes", () => {
        const a = arming();
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        a.sample({ target: "main", x: 0, y: 0, t: 400 });
        // Switching windows at t=400 must not inherit the 400ms already served.
        a.sample({ target: "floater-2", x: 0, y: 0, t: 400 });
        expect(a.isArmed()).toBe(false);
        a.sample({ target: "floater-2", x: 0, y: 0, t: 800 });
        expect(a.isArmed()).toBe(false);
        a.sample({ target: "floater-2", x: 0, y: 0, t: 900 });
        expect(a.isArmed()).toBe(true);
    });

    it("disarms and clears the indicator when the cursor leaves every target", () => {
        const a = arming();
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        a.sample({ target: "main", x: 0, y: 0, t: 600 });
        expect(a.isArmed()).toBe(true);
        const out = a.sample({ target: null, x: 0, y: 0, t: 650 });
        expect(out.armed).toBe(false);
        expect(out.clearIndicator).toBe(true);
        expect(a.isArmed()).toBe(false);
    });

    it("arms from stationary samples alone, with no cursor movement", () => {
        // The P1 heartbeat case: WM_MOUSEMOVE never fires, the 100ms drag tick
        // supplies every sample, and the cursor never moves a pixel.
        const a = arming();
        for (let t = 0; t <= 600; t += 100) {
            a.sample({ target: "main", x: 42, y: 42, t });
        }
        expect(a.isArmed()).toBe(true);
    });
});

describe("redock arming — velocity", () => {
    it("never arms during a fast transit", () => {
        const a = arming();
        // 1500 px/s across the window for a full second.
        for (let t = 0; t <= 1000; t += 50) {
            a.sample({ target: "main", x: t * 1.5, y: 0, t });
        }
        expect(a.isArmed()).toBe(false);
    });

    it("disarms and clears the indicator when an armed cursor speeds away", () => {
        const a = arming();
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        a.sample({ target: "main", x: 0, y: 0, t: 600 });
        expect(a.isArmed()).toBe(true);
        const out = a.sample({ target: "main", x: 300, y: 0, t: 650 });
        expect(out.armed).toBe(false);
        expect(out.clearIndicator).toBe(true);
    });

    it("requires a fresh full dwell after a fast entry, not merely stillness", () => {
        // This is the regression that motivated the spec. The old gate armed
        // whenever events simply went quiet for REDOCK_DWELL_MS, even though
        // the velocity gate had just rejected the entry — so a fast flick onto
        // the window followed by any pause before release would dock.
        const a = arming();
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        a.sample({ target: "main", x: 900, y: 0, t: 100 }); // 9000 px/s — rejected
        expect(a.isArmed()).toBe(false);
        // Cursor now stops dead. Stillness alone must not arm until a full
        // dwell has been served from the moment motion actually settled.
        a.sample({ target: "main", x: 900, y: 0, t: 200 });
        a.sample({ target: "main", x: 900, y: 0, t: 600 });
        expect(a.isArmed()).toBe(false);
        a.sample({ target: "main", x: 900, y: 0, t: 701 });
        expect(a.isArmed()).toBe(true);
    });

    it("does not read a cursorless teardown sample as a jump from the origin", () => {
        // clear_floating_redock_hover broadcasts { target_label: null } with no
        // cursor. Feeding 0,0 for the missing position would make the NEXT real
        // sample look like a 1000px leap and trip the velocity gate, costing an
        // extra heartbeat before the ghost could re-arm.
        const a = arming();
        a.sample({ target: "main", x: 900, y: 500, t: 0 });
        a.sample({ target: null, t: 100 }); // teardown — no cursor
        a.sample({ target: "main", x: 900, y: 500, t: 200 });
        // Position never actually changed, so dwell runs uninterrupted from 200.
        a.sample({ target: "main", x: 900, y: 500, t: 699 });
        expect(a.isArmed()).toBe(false);
        a.sample({ target: "main", x: 900, y: 500, t: 700 });
        expect(a.isArmed()).toBe(true);
    });

    it("treats a zero-duration sample as no motion rather than infinite speed", () => {
        const a = arming();
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        a.sample({ target: "main", x: 50, y: 0, t: 0 });
        // Must not throw or divide by zero, and must not have armed yet.
        expect(a.isArmed()).toBe(false);
    });
});

describe("redock arming — lifecycle", () => {
    it("reports the current target so callers can pre-capture the ghost", () => {
        const a = arming();
        expect(a.target()).toBe(null);
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        expect(a.target()).toBe("main");
    });

    it("does not arm until a sample confirms the dwell", () => {
        // Previously isArmed(now) extrapolated: with the last sample at 400ms
        // a release at 520ms armed on the wall clock alone. That let a release
        // dock before the target window had painted any ghost (it only paints
        // on an armed sample), and firstSeenAt belongs to the target of the
        // LAST sample, so the extrapolation could arm for a window the cursor
        // had already left. codex P1 on #3124.
        const a = arming();
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        a.sample({ target: "main", x: 0, y: 0, t: 400 });
        // A release at 520ms would once have armed here on the wall clock.
        expect(a.isArmed()).toBe(false);
        // The heartbeat delivers the confirming sample, and only now is it armed.
        a.sample({ target: "main", x: 0, y: 0, t: 500 });
        expect(a.isArmed()).toBe(true);
    });

    it("never reports armed for a window the cursor has already left", () => {
        // The half of codex P1 that survives requiring a confirmed sample:
        // arming is about a specific target, so a caller must be able to tell
        // WHICH one. tryRedockAtCursorInner re-resolves the window under the
        // cursor at release and compares it against target() for exactly this.
        const a = arming();
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        a.sample({ target: "main", x: 0, y: 0, t: 500 });
        expect(a.isArmed()).toBe(true);
        expect(a.target()).toBe("main");
        // Cursor flicks to another window; the next sample retargets and the
        // arming is dropped rather than transferred.
        a.sample({ target: "other", x: 0, y: 0, t: 560 });
        expect(a.isArmed()).toBe(false);
        expect(a.target()).toBe("other");
    });

    it("forgets everything on reset so a second drag cannot inherit state", () => {
        const a = arming();
        a.sample({ target: "main", x: 0, y: 0, t: 0 });
        a.sample({ target: "main", x: 0, y: 0, t: 600 });
        expect(a.isArmed()).toBe(true);
        a.reset();
        expect(a.isArmed()).toBe(false);
        expect(a.target()).toBe(null);
        // A fresh drag starting mid-clock must serve its own full dwell.
        a.sample({ target: "main", x: 0, y: 0, t: 700 });
        a.sample({ target: "main", x: 0, y: 0, t: 1000 });
        expect(a.isArmed()).toBe(false);
        a.sample({ target: "main", x: 0, y: 0, t: 1200 });
        expect(a.isArmed()).toBe(true);
    });
});
