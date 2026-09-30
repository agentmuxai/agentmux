// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tear-off picture: the window's viewport captured on pointerdown, cropped
 * to the pane, handed to the floater only if it's ready in time.
 * SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md phase 3.1.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const shots = vi.hoisted(() => ({
    calls: [] as string[],
    next: null as null | Promise<{ jpeg_base64: string }>,
    drawn: [] as number[][],
}));
vi.mock("@/app/store/global", () => ({
    getApi: () => ({
        windows: {
            captureViewport: (label: string) => {
                shots.calls.push(label);
                return shots.next ?? Promise.resolve({ jpeg_base64: btoa("full-viewport") });
            },
        },
    }),
}));

import {
    cropRectInImage,
    MAX_SNAPSHOT_CHARS,
    paneDragCandidate,
    prewarmTearOffSnapshot,
    resetTearOffSnapshotForTests,
    takeTearOffSnapshot,
} from "./tearoff-snapshot";

/** jsdom decodes no images: a 2x viewport image and a canvas that records its crop. */
function stubImagePipeline(cropped = "cropped-pane") {
    vi.stubGlobal("createImageBitmap", async () => ({
        width: window.innerWidth * 2,
        height: window.innerHeight * 2,
        close: () => {},
    }));
    vi.stubGlobal(
        "OffscreenCanvas",
        class {
            constructor(
                public width: number,
                public height: number
            ) {}
            getContext() {
                return { drawImage: (...args: unknown[]) => shots.drawn.push(args.slice(1).map(Number)) };
            }
            async convertToBlob() {
                return new Blob([cropped], { type: "image/jpeg" });
            }
        }
    );
}

function mountPane(blockId: string, rect = { left: 10, top: 20, width: 300, height: 200 }) {
    const el = document.createElement("div");
    el.setAttribute("data-blockid", blockId);
    el.getBoundingClientRect = () => ({ ...rect, right: rect.left + rect.width, bottom: rect.top + rect.height, x: rect.left, y: rect.top, toJSON() {} }) as DOMRect;
    document.body.appendChild(el);
}

beforeEach(() => {
    shots.calls = [];
    shots.next = null;
    shots.drawn = [];
    resetTearOffSnapshotForTests();
    stubImagePipeline();
});
afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
});

describe("tear-off snapshot", () => {
    it("captures the viewport and hands over the pane's crop", async () => {
        mountPane("b1");
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBe(btoa("cropped-pane"));
        // The pane's CSS rect at 2 device px per CSS px.
        expect(shots.drawn).toEqual([[20, 40, 600, 400, 0, 0, 600, 400]]);
    });

    it("a pane with no element on screen (a background pane tab) captures nothing", async () => {
        prewarmTearOffSnapshot("b1");
        expect(shots.calls).toEqual([]);
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();
    });

    it("is taken once: a second take gets nothing", async () => {
        mountPane("b1");
        prewarmTearOffSnapshot("b1");
        await takeTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();
    });

    it("never starts a capture at release", async () => {
        mountPane("b1");
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();
        expect(shots.calls).toEqual([]);
    });

    it("ignores a picture of a different pane", async () => {
        mountPane("b1");
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b2")).toBeUndefined();
    });

    it("gives up on a capture still in flight after the budget", async () => {
        vi.useFakeTimers();
        mountPane("b1");
        shots.next = new Promise(() => {});
        prewarmTearOffSnapshot("b1");
        const taken = takeTearOffSnapshot("b1");
        await vi.advanceTimersByTimeAsync(60);
        expect(await taken).toBeUndefined();
    });

    it("drops a stale picture", async () => {
        vi.useFakeTimers();
        mountPane("b1");
        prewarmTearOffSnapshot("b1");
        vi.advanceTimersByTime(10_001);
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();
    });

    it("a failed capture is no picture", async () => {
        mountPane("b1");
        shots.next = Promise.reject(new Error("no browser"));
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();
    });

    it("drops a picture too large for the open-window request", async () => {
        stubImagePipeline("x".repeat(MAX_SNAPSHOT_CHARS));
        mountPane("b1");
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();
    });
});

describe("cropRectInImage", () => {
    const image = { width: 2000, height: 1000 };

    it("scales CSS px by the image's width over the viewport's", () => {
        expect(cropRectInImage({ left: 100, top: 50, width: 300, height: 200 }, 1000, image)).toEqual({
            x: 200,
            y: 100,
            w: 600,
            h: 400,
        });
    });

    it("clamps a pane that runs off the edge", () => {
        expect(cropRectInImage({ left: 900, top: 450, width: 300, height: 200 }, 1000, image)).toEqual({
            x: 1800,
            y: 900,
            w: 200,
            h: 100,
        });
    });

    it("is nothing for a pane off screen or a zero viewport", () => {
        expect(cropRectInImage({ left: 1200, top: 0, width: 100, height: 100 }, 1000, image)).toBeNull();
        expect(cropRectInImage({ left: 0, top: 0, width: 100, height: 100 }, 0, image)).toBeNull();
    });
});

describe("paneDragCandidate", () => {
    function build(html: string): HTMLElement {
        const root = document.createElement("div");
        root.innerHTML = html;
        document.body.appendChild(root);
        return root;
    }

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
