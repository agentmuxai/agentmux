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
    page: "browser-page" as string | null,
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
                return shots.page === null ? Promise.reject(new Error("no page")) : Promise.resolve({ png_base64: shots.page });
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
    peekTearOffSnapshot,
    peekWindowTabSnapshot,
    prewarmTearOffSnapshot,
    prewarmWindowTabSnapshot,
    resetTearOffSnapshotForTests,
    settleTearOffSnapshotForTests,
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
    shots.page = "browser-page";
    resetTearOffSnapshotForTests();
    stubImagePipeline();
});
afterEach(async () => {
    vi.useRealTimers();
    // No capture a test didn't await may finish after it: one that did used to
    // throw inside jsdom after teardown (docs/reports/
    // REPORT_CI_VITEST_TEAROFF_SNAPSHOT_UNCAUGHT_2026_09_30.md).
    await settleTearOffSnapshotForTests();
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

    it("a picture older than the age limit is dropped, unless the live follow has been showing it", async () => {
        vi.useFakeTimers({ toFake: ["Date"] });
        mountPane("b1");
        prewarmTearOffSnapshot("b1", { x: 60, y: 35 });
        vi.advanceTimersByTime(30_000);
        expect(await takeTearOffSnapshot("b1")).toBeUndefined();

        vi.setSystemTime(new Date(0));
        prewarmTearOffSnapshot("b1", { x: 60, y: 35 });
        expect(await peekTearOffSnapshot("b1", 500)).toBeDefined();
        vi.advanceTimersByTime(30_000);
        expect(await takeTearOffSnapshot("b1")).toBe(btoa("cropped-pane"));
    });

    it("shows a pane's picture to the live follow with the grab and size, and leaves it for the tear-off", async () => {
        mountPane("b1", { left: 10, top: 20, width: 300, height: 200 });
        prewarmTearOffSnapshot("b1", { x: 60, y: 35 });
        expect(await peekTearOffSnapshot("b1", 500)).toEqual({
            picture: btoa("cropped-pane"),
            grab: { x: 50, y: 15 },
            size: { width: 300, height: 200 },
        });
        expect(await takeTearOffSnapshot("b1")).toBe(btoa("cropped-pane"));
    });

    it("has no pane picture to follow without the press position, or for another pane", async () => {
        mountPane("b1");
        prewarmTearOffSnapshot("b1");
        expect(await peekTearOffSnapshot("b1", 50)).toBeUndefined();
        prewarmTearOffSnapshot("b1", { x: 1, y: 1 });
        expect(await peekTearOffSnapshot("b2", 50)).toBeUndefined();
    });

    /** A browser pane at 10,20 300x200 whose page's placeholder is drawn at 10,60 300x160 (under a 40px header and address bar). */
    function mountBrowserPane(blockId: string) {
        mountPane(blockId);
        shots.views[blockId] = "browser";
        const ph = document.createElement("div");
        ph.className = "browser-placeholder";
        ph.checkVisibility = () => true;
        ph.getBoundingClientRect = () => ({ left: 10, top: 60, width: 300, height: 160, right: 310, bottom: 220, x: 10, y: 60, toJSON() {} }) as DOMRect;
        document.querySelector(`[data-blockid="${blockId}"]`)!.appendChild(ph);
    }

    it("a native browser pane is the window's crop (its header and address bar) with its own page drawn where the page stands", async () => {
        mountBrowserPane("b1");
        shots.page = btoa("page-jpeg");
        prewarmTearOffSnapshot("b1", { x: 60, y: 35 });
        expect(await peekTearOffSnapshot("b1", 500)).toEqual({
            picture: btoa("cropped-pane"),
            grab: { x: 50, y: 15 },
            size: { width: 300, height: 200 },
        });
        expect(shots.calls.sort()).toEqual(["browser:b1", "main"]);
        const page = [window.innerWidth * 2, window.innerHeight * 2];
        expect(shots.drawn).toEqual([
            // The pane, at 2 device px per CSS px.
            [20, 40, 600, 400, 0, 0, 600, 400],
            // The whole page picture into the placeholder: 40 CSS px down the pane, 300 by 160.
            [0, 0, ...page, 0, 80, 600, 320],
        ]);
    });

    it("a browser pane whose page gives no picture (asleep, not loaded) is the pane as the window shows it", async () => {
        mountBrowserPane("b1");
        shots.page = null;
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBe(btoa("cropped-pane"));
        expect(shots.drawn).toEqual([[20, 40, 600, 400, 0, 0, 600, 400]]);
    });

    it("a browser pane the window could not picture, or with no placeholder drawn, is its page alone", async () => {
        mountBrowserPane("b1");
        shots.next = Promise.reject(new Error("no capture"));
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBe("browser-page");
        shots.next = null;
        mountPane("b2");
        shots.views.b2 = "browser";
        prewarmTearOffSnapshot("b2");
        expect(await takeTearOffSnapshot("b2")).toBe("browser-page");
        expect(shots.drawn).toEqual([]);
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

    // The CI flake's throw site was jsdom's FileReader; the encode must not use it.
    it("encodes without FileReader", async () => {
        vi.stubGlobal(
            "FileReader",
            class {
                readAsDataURL() {
                    throw new Error("FileReader must not be used");
                }
            }
        );
        mountPane("b1");
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b1")).toBe(btoa("cropped-pane"));
    });

    it("settling waits for an encode the test didn't await", async () => {
        let encoded = false;
        vi.stubGlobal(
            "OffscreenCanvas",
            class {
                getContext() {
                    return { drawImage: () => {} };
                }
                async convertToBlob() {
                    const blob = new Blob(["slow"], { type: "image/jpeg" });
                    const read = blob.arrayBuffer.bind(blob);
                    blob.arrayBuffer = async () => {
                        await new Promise((r) => setTimeout(r, 30));
                        encoded = true;
                        return read();
                    };
                    return blob;
                }
            }
        );
        mountPane("b1");
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b2")).toBeUndefined();
        await settleTearOffSnapshotForTests();
        expect(encoded).toBe(true);
    });

    it("ignores a picture of a different pane", async () => {
        mountPane("b1");
        prewarmTearOffSnapshot("b1");
        expect(await takeTearOffSnapshot("b2")).toBeUndefined();
    });

    it("gives up on a capture still in flight after the budget", async () => {
        vi.useFakeTimers();
        mountPane("b1");
        // Never answers. afterEach's settle must not wait on it: a stalled
        // RPC is never tracked (Codex P2 on #4110).
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
    it("shows the picture to the live follow without taking it from the tear-off", async () => {
        prewarmWindowTabSnapshot("t1");
        expect(await peekWindowTabSnapshot("t1", 500)).toBe(btoa("full-viewport"));
        expect(await peekWindowTabSnapshot("t1", 500)).toBe(btoa("full-viewport"));
        expect(await takeWindowTabSnapshot("t1")).toBe(btoa("full-viewport"));
    });

    it("has no picture for another tab, or none held, or one that is not ready in the wait", async () => {
        expect(await peekWindowTabSnapshot("t1", 10)).toBeUndefined();
        prewarmWindowTabSnapshot("t1");
        expect(await peekWindowTabSnapshot("t2", 10)).toBeUndefined();
        resetTearOffSnapshotForTests();
        shots.next = new Promise(() => {});
        prewarmWindowTabSnapshot("t1");
        expect(await peekWindowTabSnapshot("t1", 10)).toBeUndefined();
    });

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
