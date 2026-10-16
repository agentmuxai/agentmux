// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { AgentDormancyProvider } from "../agent-dormancy";
import { SHOW_COVER_MAX_MS, useSettleOnShow, type ShowCoverDeps } from "./settle-on-show";

function mount(following: boolean, dormantAtMount = true) {
    const [dormant, setDormant] = createSignal(dormantAtMount);
    const flow = { settleOnShow: vi.fn() };
    const Probe = () => {
        useSettleOnShow(flow, () => following);
        return null;
    };
    render(() => (
        <AgentDormancyProvider dormant={dormant}>
            <Probe />
        </AgentDormancyProvider>
    ));
    return { flow, setDormant };
}

// SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §3.1.
describe("useSettleOnShow", () => {
    it("settles when a following pane is shown again", () => {
        const { flow, setDormant } = mount(true);
        expect(flow.settleOnShow).not.toHaveBeenCalled(); // not at mount
        setDormant(false);
        expect(flow.settleOnShow).toHaveBeenCalledTimes(1);
        setDormant(true); // hidden: nothing
        expect(flow.settleOnShow).toHaveBeenCalledTimes(1);
        setDormant(false);
        expect(flow.settleOnShow).toHaveBeenCalledTimes(2);
    });

    it("leaves a pane the user scrolled up alone", () => {
        const { flow, setDormant } = mount(false);
        setDormant(false);
        expect(flow.settleOnShow).not.toHaveBeenCalled();
    });

    it("does nothing without a one-way flow", () => {
        expect(() => render(() => <AgentDormancyProvider dormant={() => false}>{(() => { useSettleOnShow(null, () => true); return null; })()}</AgentDormancyProvider>)).not.toThrow();
    });
});

/** A pane with a cover, driven frame by frame. */
function mountCovered(following = true) {
    const [dormant, setDormant] = createSignal(false);
    const [changes, setChanges] = createSignal(0);
    const frames: (() => void)[] = [];
    let now = 0;
    let onResize: (() => void) | null = null;
    const observed: Element[] = [];
    const deps: ShowCoverDeps = {
        frame: (cb) => frames.push(cb),
        cancelFrame: (id) => {
            frames[id - 1] = () => {};
        },
        now: () => now,
        observer: (cb) => {
            onResize = cb;
            return { observe: (el) => observed.push(el), disconnect: () => (onResize = null) };
        },
    };
    let covered!: () => boolean;
    const el = document.createElement("div");
    const Probe = () => {
        covered = useSettleOnShow({ settleOnShow: vi.fn() }, () => following, { changes, observe: () => [el, undefined] }, deps);
        return null;
    };
    render(() => (
        <AgentDormancyProvider dormant={dormant}>
            <Probe />
        </AgentDormancyProvider>
    ));
    let ran = 0;
    /** Runs the next frame: its check, then (when `resized`) its layout's resize report. */
    const frame = (ms: number, resized = false) => {
        now += ms;
        frames[ran++]?.();
        if (resized) onResize?.();
    };
    return { covered: () => covered(), setDormant, setChanges, frame, observed, observing: () => onResize != null };
}

// SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §3.7.
describe("useSettleOnShow's cover", () => {
    it("covers a pane shown with content it got while hidden until its size holds still", () => {
        const p = mountCovered();
        p.setDormant(true);
        p.setChanges(1); // output while hidden
        p.setDormant(false);
        expect(p.covered()).toBe(true);
        expect(p.observed.length).toBe(1);
        // Each frame's check sees the previous frame's layout.
        p.frame(300, true); // the first frame: long, and the content resizes
        p.frame(16, true); // a second stage
        p.frame(16); // laid out still
        expect(p.covered()).toBe(true);
        p.frame(16); // laid out still twice
        expect(p.covered()).toBe(true);
        p.frame(16); // shown in this frame
        expect(p.covered()).toBe(false);
        expect(p.observing()).toBe(false);
    });

    it("shows a pane with nothing new at once", () => {
        const p = mountCovered();
        p.setDormant(true);
        p.setDormant(false);
        expect(p.covered()).toBe(false);
    });

    it("lifts after SHOW_COVER_MAX_MS from the first frame even while content keeps resizing", () => {
        const p = mountCovered();
        p.setDormant(true);
        p.setChanges(1);
        p.setDormant(false);
        p.frame(500, true); // the time before the first frame doesn't count
        let elapsed = 0;
        while (p.covered() && elapsed < 1000) {
            p.frame(16, true);
            elapsed += 16;
        }
        expect(elapsed).toBeGreaterThanOrEqual(SHOW_COVER_MAX_MS);
        expect(elapsed).toBeLessThan(SHOW_COVER_MAX_MS + 16);
    });

    it("never covers a pane the user scrolled up", () => {
        const p = mountCovered(false);
        p.setDormant(true);
        p.setChanges(1);
        p.setDormant(false);
        expect(p.covered()).toBe(false);
    });

    it("lets go when the pane is hidden again while covered", () => {
        const p = mountCovered();
        p.setDormant(true);
        p.setChanges(1);
        p.setDormant(false);
        expect(p.covered()).toBe(true);
        p.setDormant(true);
        expect(p.covered()).toBe(false);
        expect(p.observing()).toBe(false);
    });
});
