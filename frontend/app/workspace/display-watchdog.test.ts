// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createDisplayWatchdog, OFFER_RELOAD_AFTER_MS, SOFT_AFTER_MS } from "./display-watchdog";

function setup(over: { inFlight?: boolean } = {}) {
    let t = 1_000_000;
    const state = { expected: "B", shown: "A" as string | null, inFlight: over.inFlight ?? false };
    const repair = vi.fn((tab: string) => {
        state.shown = tab;
    });
    const offerReload = vi.fn();
    const withdrawOffer = vi.fn();
    const log = vi.fn();
    const w = createDisplayWatchdog({
        expected: () => state.expected,
        shownInDom: () => state.shown,
        inFlight: () => state.inFlight,
        repair,
        offerReload,
        withdrawOffer,
        log,
        now: () => t,
    });
    const advance = (ms: number) => {
        t += ms;
        w.check();
    };
    return { state, w, advance, repair, offerReload, withdrawOffer, log };
}

describe("the tab display watchdog", () => {
    it("does nothing while the page shows the tab that should be shown", () => {
        const { state, w, advance, repair, offerReload } = setup();
        state.shown = "B";
        w.check();
        advance(60_000);
        expect(repair).not.toHaveBeenCalled();
        expect(offerReload).not.toHaveBeenCalled();
    });

    it("does nothing while a switch is in flight, however long it takes", () => {
        const { w, advance, repair, offerReload } = setup({ inFlight: true });
        w.check();
        advance(60_000);
        expect(repair).not.toHaveBeenCalled();
        expect(offerReload).not.toHaveBeenCalled();
    });

    it("does nothing while no tab is shown: a tab closing, opening or being revealed", () => {
        const { state, w, advance, repair, offerReload } = setup();
        state.shown = null;
        w.check();
        advance(60_000);
        expect(repair).not.toHaveBeenCalled();
        expect(offerReload).not.toHaveBeenCalled();
    });

    it("waits out a mismatch shorter than the soft delay", () => {
        const { w, advance, repair } = setup();
        w.check();
        advance(SOFT_AFTER_MS - 1);
        expect(repair).not.toHaveBeenCalled();
    });

    it("shows the expected tab after the soft delay, and logs it", () => {
        const { w, advance, repair, offerReload, log } = setup();
        w.check();
        advance(SOFT_AFTER_MS);
        expect(repair).toHaveBeenCalledWith("B");
        expect(log).toHaveBeenCalledTimes(1);
        expect(log.mock.calls[0][0]).toContain("showing A for 2000 ms while B is the active tab");
        expect(offerReload).not.toHaveBeenCalled();
    });

    it("offers a reload when the repair did not take, and not before", () => {
        const { state, w, advance, offerReload, log } = setup();
        w.check();
        advance(SOFT_AFTER_MS);
        state.shown = "A"; // the display signal moved but the page did not follow
        advance(OFFER_RELOAD_AFTER_MS - SOFT_AFTER_MS - 1);
        expect(offerReload).not.toHaveBeenCalled();
        advance(1);
        expect(offerReload).toHaveBeenCalledTimes(1);
        expect(log.mock.calls.at(-1)?.[0]).toContain("offering a reload");
    });

    it("offers once, however long the window stays stuck, and never reloads by itself", () => {
        const { state, w, advance, offerReload } = setup();
        w.check();
        advance(SOFT_AFTER_MS);
        state.shown = "A";
        advance(OFFER_RELOAD_AFTER_MS);
        advance(60_000);
        advance(60_000);
        expect(offerReload).toHaveBeenCalledTimes(1);
    });

    it("takes the offer back once the page shows the right tab again", () => {
        const { state, w, advance, offerReload, withdrawOffer } = setup();
        w.check();
        advance(SOFT_AFTER_MS);
        state.shown = "A";
        advance(OFFER_RELOAD_AFTER_MS);
        expect(offerReload).toHaveBeenCalledTimes(1);
        expect(withdrawOffer).not.toHaveBeenCalled();
        state.shown = "B";
        w.check();
        expect(withdrawOffer).toHaveBeenCalledTimes(1);
        w.check();
        expect(withdrawOffer).toHaveBeenCalledTimes(1);
    });

    it("starts over once the tab shown agrees again", () => {
        const { state, w, advance, repair } = setup();
        w.check();
        advance(SOFT_AFTER_MS);
        expect(repair).toHaveBeenCalledTimes(1);
        state.shown = "B";
        w.check();
        state.expected = "C";
        state.shown = "B";
        w.check();
        advance(SOFT_AFTER_MS);
        expect(repair).toHaveBeenCalledTimes(2);
        expect(repair).toHaveBeenLastCalledWith("C");
    });
});
