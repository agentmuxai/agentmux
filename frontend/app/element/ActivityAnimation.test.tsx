// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A bucket's animation: frames on the shared ticker, its own still under reduced motion, one timer for all. */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const [reduced, setReduced] = createSignal(false);
vi.mock("@/app/motion/motion", () => ({ shouldReduceMotion: (category: string) => category === "status" && reduced() }));

import { BUCKETS } from "./activity-bucket";
import { ActivityAnimation } from "./ActivityAnimation";
import { AsciiSpinner, SPINNER_FRAMES } from "./AsciiSpinner";
import { TICKER_MS } from "./shared-ticker";

describe("ActivityAnimation", () => {
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
        setReduced(false);
    });

    it("steps through its frames, each 3 cells, on the shared 120 ms ticker", () => {
        vi.useFakeTimers();
        const { container } = render(() => <ActivityAnimation bucket={BUCKETS.test} />);
        const glyph = () => container.querySelector(".activity-animation")!.textContent;
        const seen = [glyph()];
        for (let i = 0; i < 8; i++) {
            vi.advanceTimersByTime(TICKER_MS);
            seen.push(glyph());
        }
        // 240 ms a frame: two ticks each.
        expect(seen).toEqual(["v  ", "v  ", "vv ", "vv ", "vvv", "vvv", "   ", "   ", "v  "]);
    });

    it("carries its bucket and is hidden from assistive technology", () => {
        const { container } = render(() => <ActivityAnimation bucket={BUCKETS.deploy} />);
        const el = container.querySelector(".activity-animation")!;
        expect(el.getAttribute("data-bucket")).toBe("deploy");
        expect(el.getAttribute("aria-hidden")).toBe("true");
    });

    it("shows its own still glyph when the motion policy reduces status, so the kind stays readable", () => {
        vi.useFakeTimers();
        setReduced(true);
        const { container } = render(() => (
            <>
                <ActivityAnimation bucket={BUCKETS.build} />
                <ActivityAnimation bucket={BUCKETS.hold} />
            </>
        ));
        vi.advanceTimersByTime(TICKER_MS * 5);
        const glyphs = [...container.querySelectorAll(".activity-animation")].map((e) => e.textContent);
        expect(glyphs).toEqual(["+x+", "zZz"]);
        expect(vi.getTimerCount()).toBe(0);
    });

    it("shares one timer with the slash and with every other animation, and identical ones move in step", () => {
        vi.useFakeTimers();
        const { container } = render(() => (
            <>
                <AsciiSpinner />
                <ActivityAnimation bucket={BUCKETS.search} />
                <ActivityAnimation bucket={BUCKETS.search} />
                <ActivityAnimation bucket={BUCKETS.wait} />
            </>
        ));
        expect(vi.getTimerCount()).toBe(1);
        vi.advanceTimersByTime(TICKER_MS * 2);
        const glyphs = [...container.querySelectorAll(".activity-animation")].map((e) => e.textContent);
        expect(glyphs[0]).toBe(glyphs[1]);
        expect(glyphs[0]).toBe("  o");
        expect(container.querySelector(".agent-spinner-ascii")!.textContent).toBe(SPINNER_FRAMES[2]);
    });

    it("lets the timer go with the last animation", () => {
        vi.useFakeTimers();
        const a = render(() => <ActivityAnimation bucket={BUCKETS.other} />);
        const b = render(() => <AsciiSpinner />);
        a.unmount();
        expect(vi.getTimerCount()).toBe(1);
        b.unmount();
        expect(vi.getTimerCount()).toBe(0);
    });
});
