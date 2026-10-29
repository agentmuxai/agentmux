// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { observePaneElement } from "./pane-resize-observer";

/** A ResizeObserver the test fires by hand. */
class FakeObserver {
    static all: FakeObserver[] = [];
    observed = new Set<Element>();
    disconnected = false;
    constructor(public cb: (entries: { target: Element }[]) => void) {
        FakeObserver.all.push(this);
    }
    observe(el: Element) {
        this.observed.add(el);
    }
    unobserve(el: Element) {
        this.observed.delete(el);
    }
    disconnect() {
        this.disconnected = true;
    }
    fire(...els: Element[]) {
        this.cb(els.map((target) => ({ target })));
    }
}

// The tests run without a DOM; the observer only needs elements to tell apart.
const el = () => ({}) as Element;

afterEach(() => {
    FakeObserver.all = [];
    vi.unstubAllGlobals();
});

describe("observePaneElement", () => {
    it("handles every pane the window's observer delivers, then sends once", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        const [a, b] = [el(), el()];
        const order: string[] = [];
        const send = () => order.push("send");
        observePaneElement("w-both", a, () => order.push("a"), send);
        observePaneElement("w-both", b, () => order.push("b"), send);
        expect(FakeObserver.all).toHaveLength(1);
        FakeObserver.all[0].fire(a, b);
        expect(order).toEqual(["a", "b", "send"]);
    });

    it("gives each window its own observer", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        observePaneElement("w-one", el(), () => {}, () => {});
        observePaneElement("w-two", el(), () => {}, () => {});
        expect(FakeObserver.all).toHaveLength(2);
    });

    it("stops watching a pane that is gone, and lets go of the observer with the last one", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        const [a, b] = [el(), el()];
        const seen: string[] = [];
        const stopA = observePaneElement("w-stop", a, () => seen.push("a"), () => {});
        const stopB = observePaneElement("w-stop", b, () => seen.push("b"), () => {});
        const obs = FakeObserver.all[0];
        stopA();
        obs.fire(a, b);
        expect(seen).toEqual(["b"]);
        expect(obs.disconnected).toBe(false);
        stopB();
        expect(obs.disconnected).toBe(true);
    });
});
