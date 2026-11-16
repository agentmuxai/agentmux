// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The ticker's reading of the marks on screen (#47754): how many animate, how many stopped following the clock. */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AsciiSpinner } from "./AsciiSpinner";
import { acquireTicker, markHealth, registerMark, TICKER_MS } from "./shared-ticker";

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
        expect(h.tick).toBeGreaterThan(30);
    });

    it("counts a mark that stopped drawing as frozen, and drops it when it is released", () => {
        const release = acquireTicker();
        const stuck = registerMark();
        const moving = registerMark();
        for (let i = 0; i < 20; i++) {
            vi.advanceTimersByTime(TICKER_MS);
            moving.drew(markHealth().tick);
        }
        expect(markHealth()).toMatchObject({ marks: 2, frozen: 1 });
        stuck.release();
        moving.release();
        expect(markHealth().marks).toBe(0);
        release();
    });

    it("a spinner that goes leaves the count", () => {
        const view = render(() => <AsciiSpinner />);
        expect(markHealth().marks).toBe(1);
        view.unmount();
        expect(markHealth().marks).toBe(0);
    });
});
