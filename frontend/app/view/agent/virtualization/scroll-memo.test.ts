// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { recycleBlockers } from "@/app/store/recycle-blockers";
import { captureScrollMemo, noteOlderHistoryLoaded, saveScrollMemos, takeScrollMemo, useScrollMemo } from "./scroll-memo";

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
        // Rows at 30..80, 80..130, 130..180: the first to start on screen (top 100) is c, 30 px below it.
        expect(captureScrollMemo(scroller(["a", "b", "c"], 30), false)).toEqual({ nodeId: "c", offsetPx: -30 });
        expect(captureScrollMemo(undefined, false)).toBeNull();
    });

    it("uses the row cut by the top edge only when it covers the whole screen", () => {
        // One tall row from 40 to 800 covers the screen (100..400): 60 px into it.
        const el = document.createElement("div");
        el.getBoundingClientRect = () => ({ top: 100, bottom: 400 }) as DOMRect;
        const tall = document.createElement("div");
        tall.dataset.nodeId = "tall";
        tall.getBoundingClientRect = () => ({ top: 40, bottom: 800 }) as DOMRect;
        const next = document.createElement("div");
        next.dataset.nodeId = "next";
        next.getBoundingClientRect = () => ({ top: 800, bottom: 850 }) as DOMRect;
        el.append(tall, next);
        expect(captureScrollMemo(el, false)).toEqual({ nodeId: "tall", offsetPx: 60 });
    });

    it("stores the offset unzoomed: getBoundingClientRect is zoomed under the pane's CSS zoom", () => {
        // At 2x, c starts 30 zoomed px below the top: 15 px of scrollTop.
        expect(captureScrollMemo(scroller(["a", "b", "c"], 30), false, 2)).toEqual({ nodeId: "c", offsetPx: -15 });
        expect(captureScrollMemo(scroller(["a", "b", "c"], 30), false, 0.5)).toEqual({ nodeId: "c", offsetPx: -60 });
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
        expect(JSON.parse(sessionStorage.getItem("agentmux:scroll:b1")!)).toEqual({ nodeId: "c", offsetPx: -30 });
        dispose();

        // The reloaded page mounts the pane again.
        const [ready2, setReady2] = createSignal(false);
        createRoot(() =>
            useScrollMemo({ blockId: "b1", scroller: () => undefined, following: () => true, historyReady: ready2, scrollToRow })
        );
        expect(scrollToRow).not.toHaveBeenCalled();
        setReady2(true);
        expect(scrollToRow).toHaveBeenCalledWith("c", -30);
        // Held for a few frames while the rows above settle, always the same row and offset.
        expect(scrollToRow.mock.calls.every(([id, off]) => id === "c" && off === -30)).toBe(true);
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

    it("a pane scrolled up into older history it loaded blocks a renderer recycle", () => {
        const reason = "an agent pane is scrolled up into older history";
        const [following, setFollowing] = createSignal(false);
        const dispose = createRoot((d) => {
            useScrollMemo({ blockId: "b-older", scroller: () => undefined, following, historyReady: () => false, scrollToRow: vi.fn() });
            return d;
        });
        expect(recycleBlockers()).not.toContain(reason); // no older page loaded yet
        noteOlderHistoryLoaded("b-older");
        expect(recycleBlockers()).toContain(reason);
        setFollowing(true);
        expect(recycleBlockers()).not.toContain(reason); // back at the bottom
        setFollowing(false);
        dispose();
        expect(recycleBlockers()).not.toContain(reason);
    });
});
