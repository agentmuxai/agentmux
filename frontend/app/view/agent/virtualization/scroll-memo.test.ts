// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { captureScrollMemo, saveScrollMemos, takeScrollMemo, useScrollMemo } from "./scroll-memo";

/** A scroller whose top is at 100 px, with rows of 50 px from `firstTop`. */
function scroller(ids: string[], firstTop: number): HTMLElement {
    const el = document.createElement("div");
    el.getBoundingClientRect = () => ({ top: 100, bottom: 400 }) as DOMRect;
    ids.forEach((id, i) => {
        const row = document.createElement("div");
        row.dataset.nodeId = id;
        const top = firstTop + i * 50;
        row.getBoundingClientRect = () => ({ top, bottom: top + 50 }) as DOMRect;
        el.appendChild(row);
    });
    return el;
}

describe("scroll memo (SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §2)", () => {
    beforeEach(() => {
        sessionStorage.clear();
        vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
            cb();
            return 1;
        });
    });
    afterEach(() => vi.unstubAllGlobals());

    it("records a following pane as at the bottom, else the top row and the offset into it", () => {
        expect(captureScrollMemo(scroller(["a"], 0), true)).toEqual({ bottom: true });
        // Rows at 30..80, 80..130, 130..180: the first one below the top (100) is b, 20 px in.
        expect(captureScrollMemo(scroller(["a", "b", "c"], 30), false)).toEqual({ nodeId: "b", offsetPx: 20 });
        expect(captureScrollMemo(undefined, false)).toBeNull();
    });

    it("stores the offset unzoomed: getBoundingClientRect is zoomed under the pane's CSS zoom", () => {
        // At 2x, b is 20 zoomed px in: 10 px of scrollTop.
        expect(captureScrollMemo(scroller(["a", "b", "c"], 30), false, 2)).toEqual({ nodeId: "b", offsetPx: 10 });
        expect(captureScrollMemo(scroller(["a", "b", "c"], 30), false, 0.5)).toEqual({ nodeId: "b", offsetPx: 40 });
    });

    it("is read once, and ignores what it can't use", () => {
        sessionStorage.setItem("agentmux:scroll:b1", JSON.stringify({ nodeId: "n1", offsetPx: 12 }));
        expect(takeScrollMemo("b1")).toEqual({ nodeId: "n1", offsetPx: 12 });
        expect(takeScrollMemo("b1")).toBeUndefined();
        sessionStorage.setItem("agentmux:scroll:b2", JSON.stringify({ nodeId: 3 }));
        expect(takeScrollMemo("b2")).toBeUndefined();
    });

    it("records every mounted pane when the page goes away, and puts the position back once its history loads", () => {
        const scrollToRow = vi.fn();
        const [ready] = createSignal(false);
        const dispose = createRoot((d) => {
            useScrollMemo({
                blockId: "b1",
                scroller: () => scroller(["a", "b", "c"], 30),
                following: () => false,
                historyReady: ready,
                scrollToRow,
            });
            return d;
        });
        window.dispatchEvent(new Event("pagehide")); // the reload
        expect(JSON.parse(sessionStorage.getItem("agentmux:scroll:b1")!)).toEqual({ nodeId: "b", offsetPx: 20 });
        dispose();

        // The reloaded page mounts the pane again.
        const [ready2, setReady2] = createSignal(false);
        createRoot(() =>
            useScrollMemo({ blockId: "b1", scroller: () => undefined, following: () => true, historyReady: ready2, scrollToRow })
        );
        expect(scrollToRow).not.toHaveBeenCalled();
        setReady2(true);
        expect(scrollToRow).toHaveBeenCalledWith("b", 20);
        // Held for a few frames while the rows above settle, always the same row and offset.
        expect(scrollToRow.mock.calls.every(([id, off]) => id === "b" && off === 20)).toBe(true);
        const held = scrollToRow.mock.calls.length;
        setReady2(false);
        setReady2(true);
        expect(scrollToRow).toHaveBeenCalledTimes(held); // restored once, not again
    });

    it("stops holding the restored position as soon as the user touches the pane", () => {
        sessionStorage.setItem("agentmux:scroll:b1", JSON.stringify({ nodeId: "n1", offsetPx: 5 }));
        const frames: (() => void)[] = [];
        vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
            frames.push(cb);
            return frames.length;
        });
        const el = document.createElement("div");
        const scrollToRow = vi.fn();
        createRoot(() =>
            useScrollMemo({ blockId: "b1", scroller: () => el, following: () => false, historyReady: () => true, scrollToRow })
        );
        frames.shift()!(); // first frame: put back
        frames.shift()!(); // second: held
        expect(scrollToRow).toHaveBeenCalledTimes(2);
        el.dispatchEvent(new Event("wheel"));
        while (frames.length) frames.shift()!();
        expect(scrollToRow).toHaveBeenCalledTimes(2);
    });

    it("a pane that was at the bottom stays there (nothing to do)", () => {
        sessionStorage.setItem("agentmux:scroll:b1", JSON.stringify({ bottom: true }));
        const scrollToRow = vi.fn();
        createRoot(() =>
            useScrollMemo({ blockId: "b1", scroller: () => undefined, following: () => true, historyReady: () => true, scrollToRow })
        );
        expect(scrollToRow).not.toHaveBeenCalled();
    });

    it("saveScrollMemos records on demand (the renderer recycle)", () => {
        createRoot(() =>
            useScrollMemo({ blockId: "b9", scroller: () => undefined, following: () => true, historyReady: () => false, scrollToRow: vi.fn() })
        );
        saveScrollMemos();
        expect(JSON.parse(sessionStorage.getItem("agentmux:scroll:b9")!)).toEqual({ bottom: true });
    });
});
