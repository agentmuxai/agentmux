// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tear-off picture: the window's viewport captured on pointerdown, cropped
 * to the pane, handed to the floater only if it's ready in time.
 * SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md phase 3.1.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const shots = vi.hoisted(() => ({
    views: {} as Record<string, string>,
    calls: [] as string[],
    next: null as null | Promise<{ jpeg_base64: string }>,
    drawn: [] as number[][],
}));
vi.mock("@/app/store/global", () => ({
    MOS: {
        makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
        getObjectValue: (oref: string) => {
            const view = shots.views[oref.slice("block:".length)];
            return view ? { meta: { view } } : undefined;
        },
    },
    getApi: () => ({
        browserPanes: {
            screenshot: (blockId: string) => {
                shots.calls.push(`browser:${blockId}`);
                return Promise.resolve({ png_base64: "browser-page" });
            },
        },
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
    prewarmWindowTabSnapshot,
    resetTearOffSnapshotForTests,
    takeTearOffSnapshot,
    takeWindowTabSnapshot,
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
    shots.views = {};
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

    it("a native browser pane is captured from its own page, not the window's", async () => {
        mountPane("b1");
        shots.views.b1 = "browser";
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBe("browser-page");
        expect(shots.calls).toEqual(["browser:b1"]);
    });

    it("a background browser tab mounted in the same pane doesn't make the active tab a browser", async () => {
        mountPane("b1");
        shots.views.b1 = "term";
        document.querySelector('[data-blockid="b1"]')!.innerHTML = `<div class="browser-view"><div class="browser-placeholder"></div></div>`;
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBe(btoa("cropped-pane"));
        expect(shots.calls).toEqual(["main"]);
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

describe("window-tab snapshot", () => {
    it("hands over the whole viewport, uncropped", async () => {
        prewarmWindowTabSnapshot("t1");
        expect(await takeWindowTabSnapshot("t1")).toBe(btoa("full-viewport"));
        expect(shots.drawn).toEqual([]);
    });

    it("halves a viewport too large for the request", async () => {
        shots.next = Promise.resolve({ jpeg_base64: "xxxx".repeat(MAX_SNAPSHOT_CHARS / 4 + 1) });
        prewarmWindowTabSnapshot("t1");
        // Let the re-encode finish first: on a slow machine it can outlast
        // the take's 60 ms budget, which is the budget working, not this.
        await vi.waitFor(() => expect(shots.drawn).toHaveLength(1), { timeout: 5000 });
        expect(await takeWindowTabSnapshot("t1")).toBe(btoa("cropped-pane"));
        // Drawn at half the (2x-viewport) image's size.
        expect(shots.drawn).toEqual([[0, 0, window.innerWidth, window.innerHeight]]);
    });

    it("keeps shrinking until the picture fits, and gives up if it never does", async () => {
        const huge = "xxxx".repeat(MAX_SNAPSHOT_CHARS / 4 + 1);
        shots.next = Promise.resolve({ jpeg_base64: huge });
        stubImagePipeline(atob(huge));
        prewarmWindowTabSnapshot("t1");
        await vi.waitFor(() => expect(shots.drawn).toHaveLength(3), { timeout: 5000 });
        expect(await takeWindowTabSnapshot("t1")).toBeUndefined();
        // Tried at every scale: 1/2, 0.35, 1/4 of the (2x-viewport) image.
        expect(shots.drawn.map((d) => d[2])).toEqual([
            Math.round(window.innerWidth * 2 * 0.5),
            Math.round(window.innerWidth * 2 * 0.35),
            Math.round(window.innerWidth * 2 * 0.25),
        ]);
    });

    it("no picture of a window showing a browser pane: it would be a grey placeholder", async () => {
        const el = document.createElement("div");
        el.className = "browser-placeholder";
        el.getClientRects = () => [{}] as unknown as DOMRectList;
        document.body.appendChild(el);
        prewarmWindowTabSnapshot("t1");
        expect(shots.calls).toEqual([]);
        expect(await takeWindowTabSnapshot("t1")).toBeUndefined();
    });

    it("a browser pane in a hidden window tab or inactive pane tab (laid out, visibility:hidden) doesn't prevent it", async () => {
        const hidden = document.createElement("div");
        hidden.style.visibility = "hidden";
        const el = document.createElement("div");
        el.className = "browser-placeholder";
        el.getClientRects = () => [{}] as unknown as DOMRectList; // still laid out
        hidden.appendChild(el);
        document.body.appendChild(hidden);
        prewarmWindowTabSnapshot("t1");
        expect(await takeWindowTabSnapshot("t1")).toBe(btoa("full-viewport"));
    });

    it("uses checkVisibility() where available, e.g. content-visibility:hidden tabs", async () => {
        const el = document.createElement("div");
        el.className = "browser-placeholder";
        el.getClientRects = () => [{}] as unknown as DOMRectList; // laid out
        const check = vi.fn(() => false); // but not rendered
        (el as unknown as { checkVisibility: typeof check }).checkVisibility = check;
        document.body.appendChild(el);
        prewarmWindowTabSnapshot("t1");
        expect(check).toHaveBeenCalledWith(expect.objectContaining({ contentVisibilityAuto: true, visibilityProperty: true }));
        expect(await takeWindowTabSnapshot("t1")).toBe(btoa("full-viewport"));
    });

    it("a browser pane not laid out at all (display:none) doesn't prevent it", async () => {
        const el = document.createElement("div");
        el.className = "browser-placeholder";
        document.body.appendChild(el); // jsdom: no client rects
        prewarmWindowTabSnapshot("t1");
        expect(await takeWindowTabSnapshot("t1")).toBe(btoa("full-viewport"));
    });

    it("a window tab's picture is never handed to a pane with the same id, or vice versa", async () => {
        prewarmWindowTabSnapshot("x1");
        expect(await takeTearOffSnapshot("x1")).toBeUndefined();
        mountPane("x1");
        prewarmTearOffSnapshot("x1");
        expect(await takeWindowTabSnapshot("x1")).toBeUndefined();
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
