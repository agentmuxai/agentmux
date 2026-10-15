// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Working row's ASCII spinner: frames on its own clock, still when the motion policy reduces `status`. */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const [reduced, setReduced] = createSignal(false);
vi.mock("@/app/motion/motion", () => ({ shouldReduceMotion: (category: string) => category === "status" && reduced() }));

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

    it("takes a class to style it where it sits; the default is the Working row's", () => {
        const { container } = render(() => (
            <>
                <AsciiSpinner />
                <AsciiSpinner class="swarm-status-spinner" />
            </>
        ));
        expect(container.querySelectorAll(".agent-spinner-ascii")).toHaveLength(1);
        expect(container.querySelectorAll(".swarm-status-spinner")).toHaveLength(1);
    });

    // The Swarm can show dozens: one timer drives them all, so they step
    // together and cost one interval, not one each.
    it("every spinner shares one timer and one frame", () => {
        vi.useFakeTimers();
        const { container } = render(() => (
            <>
                <AsciiSpinner />
                <AsciiSpinner />
                <AsciiSpinner />
            </>
        ));
        expect(vi.getTimerCount()).toBe(1);
        const frames = () => [...container.querySelectorAll("span")].map((s) => s.textContent);
        expect(new Set(frames()).size).toBe(1);
        vi.advanceTimersByTime(SPINNER_FRAME_MS * 2);
        expect(new Set(frames()).size).toBe(1);
        expect(frames()[0]).toBe(SPINNER_FRAMES[2]);
    });

    it("the timer stops with the last spinner, and the next one starts from the first frame", () => {
        vi.useFakeTimers();
        const first = render(() => <AsciiSpinner />);
        const second = render(() => <AsciiSpinner />);
        vi.advanceTimersByTime(SPINNER_FRAME_MS);
        first.unmount();
        expect(vi.getTimerCount()).toBe(1); // the other still needs it
        second.unmount();
        expect(vi.getTimerCount()).toBe(0);
        const again = render(() => <AsciiSpinner />);
        expect(again.container.textContent).toBe(SPINNER_FRAMES[0]);
    });

    it("holds still when the motion policy reduces status", () => {
        vi.useFakeTimers();
        setReduced(true);
        const { container } = render(() => <AsciiSpinner />);
        vi.advanceTimersByTime(SPINNER_FRAME_MS * 5);
        expect(container.textContent).toBe(SPINNER_STILL);
    });
});
