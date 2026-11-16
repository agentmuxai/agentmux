// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one clock behind the marks, and what it reports of them (#47754: Swarm rows "working" with a frozen frame
 * each after a long session). The marks are called by the clock, so a failing flush elsewhere cannot stop them.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createMemo, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ActivityAnimation } from "./ActivityAnimation";
import { BUCKETS } from "./activity-bucket";
import { AsciiSpinner, SPINNER_FRAME_MS } from "./AsciiSpinner";
import { markHealth, subscribeFrames, TICKER_MS } from "./shared-ticker";

describe("markHealth", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("counts the spinners on screen, none frozen while the clock drives them", () => {
        render(() => (
            <>
                <AsciiSpinner />
                <AsciiSpinner />
                <AsciiSpinner />
            </>
        ));
        vi.advanceTimersByTime(TICKER_MS * 40);
        const h = markHealth();
        expect(h.marks).toBe(3);
        expect(h.frozen).toBe(0);
        expect(h.tick).toBe(40);
    });

    it("counts a mark that keeps failing as frozen, and drops it when it is released", () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        const stopBad = subscribeFrames(() => {
            throw new Error("Stale read from <Show>.");
        });
        const stopGood = subscribeFrames(() => {});
        vi.advanceTimersByTime(TICKER_MS * 20);
        expect(markHealth()).toMatchObject({ marks: 2, frozen: 1 });
        stopBad();
        stopGood();
        expect(markHealth().marks).toBe(0);
        expect(vi.getTimerCount()).toBe(0);
    });

    it("a spinner that goes leaves the count", () => {
        const view = render(() => <AsciiSpinner />);
        expect(markHealth().marks).toBe(1);
        view.unmount();
        expect(markHealth().marks).toBe(0);
    });
});

describe("marks keep moving when another computation throws in a flush", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
        vi.restoreAllMocks();
    });

    it("spinners and animations keep stepping after a memo throws and its flush is abandoned", () => {
        const [fail, setFail] = createSignal(false);
        function Thrower() {
            createMemo(() => {
                if (fail()) throw new Error("Stale read from <Show>.");
            });
            return null;
        }
        const view = render(() => (
            <div>
                <AsciiSpinner />
                <Thrower />
                <AsciiSpinner />
                <ActivityAnimation bucket={BUCKETS.search} />
            </div>
        ));
        const marks = () => [...view.container.querySelectorAll("span")].map((s) => s.textContent);
        vi.advanceTimersByTime(SPINNER_FRAME_MS * 2);
        try {
            setFail(true);
        } catch {
            // the flush that throws, as in the window
        }
        // Each mark shows more than one glyph over the next eight ticks: it still moves.
        const seen: string[][] = [[], [], []];
        for (let i = 0; i < 8; i++) {
            vi.advanceTimersByTime(SPINNER_FRAME_MS);
            marks().forEach((glyph, m) => seen[m].push(glyph ?? ""));
        }
        seen.forEach((glyphs, m) => expect(new Set(glyphs).size, `mark ${m}`).toBeGreaterThan(1));
    });

    it("one mark that throws does not stop the others", () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        const seen: number[] = [];
        const stopBad = subscribeFrames(() => {
            throw new Error("Stale read from <Show>.");
        });
        const stopGood = subscribeFrames((t) => seen.push(t));
        vi.advanceTimersByTime(TICKER_MS * 3);
        stopBad();
        stopGood();
        expect(seen).toEqual([1, 2, 3]);
    });
});
