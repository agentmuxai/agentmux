// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { createOverflowReporter } from "./overflow-report";

/** A box inside an `.agent-tool-panel`, with the sizes layout would give it. */
function box(scrollHeight: number, clientHeight: number) {
    const panel = document.createElement("div");
    panel.className = "agent-tool-panel";
    const el = document.createElement("div");
    panel.appendChild(el);
    document.body.appendChild(panel);
    const g = { scrollHeight, clientHeight };
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => g.scrollHeight });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: () => g.clientHeight });
    return { panel, el, g };
}

/** A transition as `getAnimations` reports it, finished when `end()` is called. */
function animation(endTime: number) {
    let end!: () => void;
    const finished = new Promise<void>((r) => (end = r));
    const a = {
        playState: "running",
        effect: { getComputedTiming: () => ({ endTime }) },
        finished,
    };
    return { a, end };
}

afterEach(() => {
    document.body.innerHTML = "";
});

describe("createOverflowReporter", () => {
    it("says whether the box has more content than it shows, and only when that changes", () => {
        const { el, g } = box(500, 100);
        const seen: boolean[] = [];
        const { check } = createOverflowReporter({ el: () => el, hidden: () => false, onChange: () => (o) => seen.push(o) });
        check();
        check();
        expect(seen).toEqual([true]);
        g.scrollHeight = 100;
        check();
        expect(seen).toEqual([true, false]);
    });

    it("lets one pixel of rounding go: that is not something to scroll", () => {
        const { el, g } = box(101, 100);
        const seen: boolean[] = [];
        const { check } = createOverflowReporter({ el: () => el, hidden: () => false, onChange: () => (o) => seen.push(o) });
        check();
        g.scrollHeight = 102;
        check();
        expect(seen).toEqual([false, true]);
    });

    it("reads nothing while the box is in a hidden subtree, or has nowhere to report", () => {
        const { el } = box(500, 100);
        const spy = vi.spyOn(el, "scrollHeight", "get");
        const seen: boolean[] = [];
        createOverflowReporter({ el: () => el, hidden: () => true, onChange: () => (o) => seen.push(o) }).check();
        createOverflowReporter({ el: () => el, hidden: () => false, onChange: () => undefined }).check();
        createOverflowReporter({ el: () => undefined, hidden: () => false, onChange: () => (o) => seen.push(o) }).check();
        expect(spy).not.toHaveBeenCalled();
        expect(seen).toEqual([]);
    });

    it("waits for a transition in the panel, then reads the settled size", async () => {
        // Mid-transition the panel clips the box to a fraction of its height, so
        // content that fits would read as overflowing.
        const { panel, el, g } = box(100, 20);
        const t = animation(120);
        (panel as unknown as { getAnimations: () => unknown[] }).getAnimations = () => [t.a];
        const seen: boolean[] = [];
        const { check } = createOverflowReporter({ el: () => el, hidden: () => false, onChange: () => (o) => seen.push(o) });
        check();
        check();
        expect(seen).toEqual([]);
        // The transition ends; the box is now its full height.
        (panel as unknown as { getAnimations: () => unknown[] }).getAnimations = () => [];
        g.clientHeight = 100;
        t.end();
        await vi.waitFor(() => expect(seen).toEqual([false]));
    });

    it("doesn't wait for an animation that never ends", () => {
        const { panel, el } = box(500, 100);
        (panel as unknown as { getAnimations: () => unknown[] }).getAnimations = () => [animation(Number.POSITIVE_INFINITY).a];
        const seen: boolean[] = [];
        createOverflowReporter({ el: () => el, hidden: () => false, onChange: () => (o) => seen.push(o) }).check();
        expect(seen).toEqual([true]);
    });

    it("reads again after a cancelled transition", async () => {
        const { panel, el } = box(500, 100);
        let cancel!: () => void;
        const finished = new Promise<void>((_, rej) => (cancel = () => rej(new Error("AbortError"))));
        let live = true;
        (panel as unknown as { getAnimations: () => unknown[] }).getAnimations = () =>
            live ? [{ playState: "running", effect: { getComputedTiming: () => ({ endTime: 120 }) }, finished }] : [];
        const seen: boolean[] = [];
        createOverflowReporter({ el: () => el, hidden: () => false, onChange: () => (o) => seen.push(o) }).check();
        live = false;
        cancel();
        await vi.waitFor(() => expect(seen).toEqual([true]));
    });
});
