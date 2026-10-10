// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Working row's ASCII spinner: frames on its own clock, still under reduced motion. */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const [reduced, setReduced] = createSignal(false);
vi.mock("@/app/store/global", () => ({ atoms: { prefersReducedMotionAtom: () => reduced() } }));

import { AsciiSpinner, SPINNER_FRAME_MS, SPINNER_FRAMES, SPINNER_STILL } from "./AsciiSpinner";

describe("AsciiSpinner", () => {
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
        setReduced(false);
    });

    it("cycles its frames", () => {
        vi.useFakeTimers();
        const { container } = render(() => <AsciiSpinner />);
        const seen = [container.textContent];
        for (let i = 0; i < SPINNER_FRAMES.length; i++) {
            vi.advanceTimersByTime(SPINNER_FRAME_MS);
            seen.push(container.textContent);
        }
        expect(seen).toEqual([...SPINNER_FRAMES, SPINNER_FRAMES[0]]);
    });

    it("holds still under reduced motion", () => {
        vi.useFakeTimers();
        setReduced(true);
        const { container } = render(() => <AsciiSpinner />);
        vi.advanceTimersByTime(SPINNER_FRAME_MS * 5);
        expect(container.textContent).toBe(SPINNER_STILL);
    });
});
