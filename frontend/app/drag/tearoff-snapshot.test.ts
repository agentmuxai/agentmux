// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tear-off picture: taken on pointerdown, handed to the floater only if
 * it's ready in time. SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md phase 3.1.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const shots = vi.hoisted(() => ({ calls: [] as string[], next: null as null | Promise<{ png_base64: string }> }));
vi.mock("@/app/store/global", () => ({
    getApi: () => ({
        browserPanes: {
            screenshot: (blockId: string) => {
                shots.calls.push(blockId);
                return shots.next ?? Promise.resolve({ png_base64: `jpeg-of-${blockId}` });
            },
        },
    }),
}));

import {
    paneDragCandidate,
    prewarmTearOffSnapshot,
    resetTearOffSnapshotForTests,
    takeTearOffSnapshot,
} from "./tearoff-snapshot";

beforeEach(() => {
    shots.calls = [];
    shots.next = null;
    resetTearOffSnapshotForTests();
});
afterEach(() => vi.useRealTimers());

describe("tear-off snapshot", () => {
    it("hands over the picture taken for that pane", async () => {
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBe("jpeg-of-b1");
    });

    it("is taken once: a second take gets nothing", async () => {
        prewarmTearOffSnapshot("b1");
        await takeTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();
    });

    it("never starts a capture at release", async () => {
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();
        expect(shots.calls).toEqual([]);
    });

    it("ignores a picture of a different pane", async () => {
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b2")).toBeUndefined();
    });

    it("gives up on a capture still in flight after the budget", async () => {
        vi.useFakeTimers();
        shots.next = new Promise(() => {});
        prewarmTearOffSnapshot("b1");
        const taken = takeTearOffSnapshot("b1");
        await vi.advanceTimersByTimeAsync(60);
        expect(await taken).toBeUndefined();
    });

    it("drops a stale picture", async () => {
        vi.useFakeTimers();
        prewarmTearOffSnapshot("b1");
        vi.advanceTimersByTime(10_001);
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();
    });

    it("a failed capture is no picture", async () => {
        shots.next = Promise.reject(new Error("pane not found"));
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();
    });
});

describe("paneDragCandidate", () => {
    function build(html: string): HTMLElement {
        const root = document.createElement("div");
        root.innerHTML = html;
        document.body.appendChild(root);
        return root;
    }
    afterEach(() => (document.body.innerHTML = ""));

    it("a drag handle inside a pane names that pane", () => {
        const root = build(`<div data-blockid="b1"><div draggable="true"><span id="t">title</span></div></div>`);
        expect(paneDragCandidate(root.querySelector("#t"))).toBe("b1");
    });

    it("a draggable outside any pane (a window tab) is not a pane drag", () => {
        const root = build(`<div draggable="true"><span id="t">Tab 1</span></div>`);
        expect(paneDragCandidate(root.querySelector("#t"))).toBeNull();
    });

    it("a click on pane content that isn't a drag handle is not a pane drag", () => {
        const root = build(`<div data-blockid="b1"><div id="t">content</div></div>`);
        expect(paneDragCandidate(root.querySelector("#t"))).toBeNull();
    });
});
