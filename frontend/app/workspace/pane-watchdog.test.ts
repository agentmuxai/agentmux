// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createPaneWatchdog, OFFER_AFTER_MS } from "./pane-watchdog";

function setup(initial: { expected: string[]; mounted: string[]; inFlight?: boolean }) {
    const state = { expected: initial.expected, mounted: new Set(initial.mounted), inFlight: initial.inFlight ?? false, t: 1_000 };
    const offerReload = vi.fn();
    const withdrawOffer = vi.fn();
    const log = vi.fn();
    const dog = createPaneWatchdog({
        expected: () => state.expected,
        mounted: () => state.mounted,
        inFlight: () => state.inFlight,
        offerReload,
        withdrawOffer,
        log,
        now: () => state.t,
    });
    const tick = (ms: number) => {
        state.t += ms;
        dog.check();
    };
    return { state, dog, tick, offerReload, withdrawOffer, log };
}

describe("createPaneWatchdog", () => {
    it("does nothing while every pane of the layout is mounted", () => {
        const w = setup({ expected: ["a", "b"], mounted: ["a", "b"] });
        w.dog.check();
        w.tick(60_000);
        expect(w.offerReload).not.toHaveBeenCalled();
        expect(w.log).not.toHaveBeenCalled();
    });

    it("waits out a pane that is still mounting, then offers a reload once and names what is missing", () => {
        const w = setup({ expected: ["pane-aaaaaaa", "pane-bbbbbbb", "pane-ccccccc"], mounted: ["pane-aaaaaaa"] });
        w.dog.check();
        w.tick(OFFER_AFTER_MS - 1);
        expect(w.offerReload).not.toHaveBeenCalled();
        w.tick(1);
        expect(w.offerReload).toHaveBeenCalledTimes(1);
        expect(w.log).toHaveBeenCalledTimes(1);
        expect(w.log.mock.calls[0][0]).toContain("2 of 3 panes");
        expect(w.log.mock.calls[0][0]).toContain("pane-bb");
        w.tick(30_000);
        expect(w.offerReload).toHaveBeenCalledTimes(1);
    });

    it("takes the offer back when every pane shows again, and starts afresh after", () => {
        const w = setup({ expected: ["a", "b"], mounted: ["a"] });
        w.dog.check();
        w.tick(OFFER_AFTER_MS);
        expect(w.offerReload).toHaveBeenCalledTimes(1);
        w.state.mounted.add("b");
        w.tick(1000);
        expect(w.withdrawOffer).toHaveBeenCalledTimes(1);
        w.state.mounted.delete("b");
        w.tick(1000);
        expect(w.offerReload).toHaveBeenCalledTimes(1);
        w.tick(OFFER_AFTER_MS);
        expect(w.offerReload).toHaveBeenCalledTimes(2);
    });

    it("restarts the wait when the window agrees for a moment in between", () => {
        const w = setup({ expected: ["a", "b"], mounted: ["a"] });
        w.dog.check();
        w.tick(OFFER_AFTER_MS - 1000);
        w.state.mounted.add("b");
        w.tick(1000);
        w.state.mounted.delete("b");
        w.tick(1000);
        w.tick(OFFER_AFTER_MS - 1000);
        expect(w.offerReload).not.toHaveBeenCalled();
    });

    it("counts nothing while a switch is in flight, and starts its wait after", () => {
        const w = setup({ expected: ["a", "b"], mounted: ["a"], inFlight: true });
        w.dog.check();
        w.tick(OFFER_AFTER_MS * 3);
        expect(w.offerReload).not.toHaveBeenCalled();
        w.state.inFlight = false;
        w.tick(1000);
        w.tick(OFFER_AFTER_MS - 1000);
        expect(w.offerReload).not.toHaveBeenCalled();
        w.tick(1000);
        expect(w.offerReload).toHaveBeenCalledTimes(1);
    });

    it("does not count a mounted pane the layout does not hold (a floating pane)", () => {
        const w = setup({ expected: ["a"], mounted: ["a", "floater"] });
        w.dog.check();
        w.tick(OFFER_AFTER_MS * 2);
        expect(w.offerReload).not.toHaveBeenCalled();
    });

    it("does not offer for a layout with no panes yet", () => {
        const w = setup({ expected: [], mounted: [] });
        w.dog.check();
        w.tick(OFFER_AFTER_MS * 2);
        expect(w.offerReload).not.toHaveBeenCalled();
    });
});
