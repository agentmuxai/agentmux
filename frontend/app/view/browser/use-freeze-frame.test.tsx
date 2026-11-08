// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The freeze-frame a browser pane holds while a menu hides its native surface (Linux, and Windows in
 * Views pane mode): which panes it applies to, when it captures (on pointer-down of a menu anchor, or
 * when the overlay's clip arrives), what the held frame and its geometry are, how long it is held, and
 * what it hands the host to wait on.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

import type { BrowserViewModel } from "./browser-model";
import type { PaneRect } from "./use-pane-rect-sync";

const env = vi.hoisted(() => ({ screenshot: vi.fn(), views: { on: false } }));

vi.mock("@/util/app-api", () => ({
    getApi: () => ({ browserPanes: { screenshot: env.screenshot } }),
}));
vi.mock("@/app/platform/pane-backend", () => ({
    isViewsPanesOnWindows: () => env.views.on,
}));

import { useFreezeFrame, type FreezeFrame } from "./use-freeze-frame";

const LINUX = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36";
const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";

function userAgent(ua: string) {
    Object.defineProperty(navigator, "userAgent", { value: ua, configurable: true });
}

interface Setup {
    model: { blockId: string; closed: boolean };
    placeholder: HTMLDivElement | undefined;
    rect: PaneRect;
    created: boolean;
    diag: Mock<(msg: string) => void>;
}
let s: Setup;
let fz: FreezeFrame;
let unmount: () => void;

/** The pane is at (30, 50) and 600 x 400 physical pixels, so (15, 25) and 300 x 200 CSS pixels at a ratio of 2. */
const RECT: PaneRect = { x: 30, y: 50, width: 600, height: 400 };

function mount() {
    const result = render(() => {
        fz = useFreezeFrame({
            model: s.model as unknown as BrowserViewModel,
            placeholderRef: () => s.placeholder,
            paneRect: () => s.rect,
            paneCreated: () => s.created,
            diag: s.diag,
        });
        return <div />;
    });
    unmount = result.unmount;
}

const prewarm = () => window.dispatchEvent(new Event("pane-freeze-prewarm"));
const clip = (detail?: unknown) => window.dispatchEvent(new CustomEvent("pane-overlay-clip-changed", { detail }));
const flush = () => vi.advanceTimersByTimeAsync(0);

function deferredShot() {
    let resolve!: (v: { png_base64?: string | null } | null | undefined) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<{ png_base64?: string | null } | null | undefined>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    env.screenshot.mockReturnValueOnce(promise);
    return { resolve, reject };
}

beforeEach(() => {
    vi.useFakeTimers();
    userAgent(LINUX);
    env.views.on = false;
    env.screenshot.mockReset().mockResolvedValue({ png_base64: "AAAA" });
    Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
    const placeholder = document.createElement("div");
    // The placeholder box: (10, 20) and 400 x 330 CSS pixels.
    placeholder.getBoundingClientRect = () => ({ x: 10, y: 20, width: 400, height: 330 }) as DOMRect;
    s = {
        model: { blockId: "blk-1", closed: false },
        placeholder,
        rect: { ...RECT },
        created: true,
        diag: vi.fn<(msg: string) => void>(),
    };
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    delete (navigator as unknown as Record<string, unknown>).userAgent;
});

describe("which panes it applies to", () => {
    it("captures on Linux", async () => {
        mount();
        prewarm();
        await flush();
        expect(env.screenshot).toHaveBeenCalledTimes(1);
    });

    it("does nothing on macOS, where the pane stays live", async () => {
        userAgent(MAC);
        mount();
        prewarm();
        clip({ rects: [{ x: 0, y: 0, w: 1000, h: 1000 }], wait: vi.fn() });
        await flush();
        expect(env.screenshot).not.toHaveBeenCalled();
    });

    it("does nothing on Windows, except in Views pane mode", async () => {
        userAgent(WINDOWS);
        mount();
        prewarm();
        await flush();
        expect(env.screenshot).not.toHaveBeenCalled();

        env.views.on = true;
        prewarm();
        await flush();
        expect(env.screenshot).toHaveBeenCalledTimes(1);
    });

    it("does nothing for a Windows browser that also says Linux, except in Views pane mode", async () => {
        userAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Linux compatible AppleWebKit/537.36");
        mount();
        prewarm();
        await flush();
        expect(env.screenshot).not.toHaveBeenCalled();
    });

    it("does nothing without a placeholder, before the pane exists, or once the view is closed", async () => {
        s.placeholder = undefined;
        mount();
        prewarm();
        await flush();
        expect(env.screenshot).not.toHaveBeenCalled();

        s.placeholder = document.createElement("div");
        s.created = false;
        prewarm();
        await flush();
        expect(env.screenshot).not.toHaveBeenCalled();

        s.created = true;
        s.model.closed = true;
        prewarm();
        await flush();
        expect(env.screenshot).not.toHaveBeenCalled();
    });
});

describe("the prewarm on pointer-down", () => {
    it("captures the pane as a JPEG at quality 80", async () => {
        mount();
        prewarm();
        await flush();
        expect(env.screenshot).toHaveBeenCalledWith("blk-1", { format: "jpeg", quality: 80 });
    });

    it("holds the frame as a data URL, placed over the pane's own rectangle", async () => {
        mount();
        expect(fz.freezeSnapshot()).toBeNull();
        expect(fz.freezeStyle()).toEqual({});

        prewarm();
        await flush();

        expect(fz.freezeSnapshot()).toBe("data:image/jpeg;base64,AAAA");
        // (30, 50) at a ratio of 2 is (15, 25); the box is at (10, 20): 5px in and 5px down.
        // The box is 400 x 330 and the pane 300 x 200: 100px and 130px less than the placeholder.
        expect(fz.freezeStyle()).toEqual({
            left: "5px",
            top: "5px",
            width: "calc(100% - 100px)",
            height: "calc(100% - 130px)",
        });
    });

    it("uses a ratio of 1 when the window has none", async () => {
        Object.defineProperty(window, "devicePixelRatio", { value: 0, configurable: true });
        mount();
        prewarm();
        await flush();
        // (30, 50) against a box at (10, 20); 400 x 330 against 600 x 400.
        expect(fz.freezeStyle()).toEqual({
            left: "20px",
            top: "30px",
            width: "calc(100% - -200px)",
            height: "calc(100% - -70px)",
        });
    });

    it("starts one capture, not one for each pointer-down while it is in flight or held", async () => {
        const shot = deferredShot();
        mount();
        prewarm();
        prewarm();
        await flush();
        expect(env.screenshot).toHaveBeenCalledTimes(1);

        shot.resolve({ png_base64: "AAAA" });
        await vi.advanceTimersByTimeAsync(20);
        prewarm();
        await flush();
        expect(env.screenshot).toHaveBeenCalledTimes(1);
    });

    it("drops the frame after 4 seconds if no overlay lands, and not before", async () => {
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(3999);
        expect(fz.freezeSnapshot()).not.toBeNull();
        await vi.advanceTimersByTimeAsync(1);
        expect(fz.freezeSnapshot()).toBeNull();
    });

    it("starts the 4 seconds again at each pointer-down", async () => {
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(3000);
        prewarm();
        await vi.advanceTimersByTimeAsync(3000);
        expect(fz.freezeSnapshot()).not.toBeNull();
        await vi.advanceTimersByTimeAsync(1000);
        expect(fz.freezeSnapshot()).toBeNull();
    });

    it("captures again at the next pointer-down once the frame has been dropped", async () => {
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(4000);
        expect(fz.freezeSnapshot()).toBeNull();
        expect(env.screenshot).toHaveBeenCalledTimes(1);

        prewarm();
        await vi.advanceTimersByTimeAsync(20);

        expect(env.screenshot).toHaveBeenCalledTimes(2);
        expect(fz.freezeSnapshot()).not.toBeNull();
    });

    it("does not let a drop that was replaced go off later and take a newer frame", async () => {
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(100);
        // The menu leaves: the 4 seconds are replaced by 250ms.
        clip({ rects: [] });
        await vi.advanceTimersByTimeAsync(300);
        expect(fz.freezeSnapshot()).toBeNull();

        prewarm();
        await vi.advanceTimersByTimeAsync(20);
        // 4 seconds after the first pointer-down, but not yet 4 after the second.
        await vi.advanceTimersByTimeAsync(3700);
        expect(fz.freezeSnapshot()).not.toBeNull();
    });

    it("takes over a clear that was waiting for the overlay to go", async () => {
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(20);
        clip({ rects: [] });
        await vi.advanceTimersByTimeAsync(100);
        prewarm();
        await vi.advanceTimersByTimeAsync(400);
        expect(fz.freezeSnapshot()).not.toBeNull();
    });
});

describe("a capture that does not come out", () => {
    it("holds nothing when the screenshot has no image", async () => {
        env.screenshot.mockResolvedValue({ png_base64: "" });
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(20);
        expect(fz.freezeSnapshot()).toBeNull();

        env.screenshot.mockResolvedValue(null);
        prewarm();
        await vi.advanceTimersByTimeAsync(20);
        expect(fz.freezeSnapshot()).toBeNull();
    });

    it("says so, and holds nothing, when the capture fails", async () => {
        env.screenshot.mockRejectedValue(new Error("no page"));
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(20);
        expect(fz.freezeSnapshot()).toBeNull();
        expect(s.diag).toHaveBeenCalledWith("[freeze] capture failed: Error: no page");
    });

    it("does not bring back a frame for a capture that was dropped while it ran", async () => {
        const shot = deferredShot();
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(4000);
        shot.resolve({ png_base64: "AAAA" });
        await vi.advanceTimersByTimeAsync(20);
        expect(fz.freezeSnapshot()).toBeNull();
    });
});

describe("the overlay's clip", () => {
    // The pane is at (15, 25) and 300 x 200 CSS pixels: x from 15 to 315, y from 25 to 225.
    const rect = (x: number, y: number, w = 50, h = 50) => ({ x, y, w, h });

    it("waits for a capture of its own when a menu lands on the pane and nothing is held", async () => {
        const wait = vi.fn();
        mount();
        clip({ rects: [rect(100, 100)], wait });
        await flush();

        expect(env.screenshot).toHaveBeenCalledTimes(1);
        expect(wait).toHaveBeenCalledTimes(1);
        expect(wait.mock.calls[0][0]).toBeInstanceOf(Promise);
    });

    it("gives the host a wait that ends one frame after the image is placed", async () => {
        const wait = vi.fn();
        mount();
        clip({ rects: [rect(100, 100)], wait });
        let done = false;
        void (wait.mock.calls[0][0] as Promise<void>).then(() => (done = true));

        await vi.advanceTimersByTimeAsync(0);
        expect(fz.freezeSnapshot()).toBe("data:image/jpeg;base64,AAAA");
        expect(done).toBe(false);
        await vi.advanceTimersByTimeAsync(20);
        expect(done).toBe(true);
    });

    it("reuses a capture that is already in flight instead of starting another", async () => {
        const shot = deferredShot();
        const wait = vi.fn();
        mount();
        prewarm();
        await flush();
        clip({ rects: [rect(100, 100)], wait });
        await flush();

        expect(env.screenshot).toHaveBeenCalledTimes(1);
        expect(wait).toHaveBeenCalledTimes(1);
        shot.resolve({ png_base64: "AAAA" });
        await vi.advanceTimersByTimeAsync(20);
        expect(fz.freezeSnapshot()).not.toBeNull();
    });

    it("starts a new capture, not one waiting on the old, once the last frame has been dropped", async () => {
        const wait = vi.fn();
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(4000);
        expect(fz.freezeSnapshot()).toBeNull();
        env.screenshot.mockClear();

        clip({ rects: [rect(100, 100)], wait });
        await vi.advanceTimersByTimeAsync(20);

        expect(env.screenshot).toHaveBeenCalledTimes(1);
        expect(fz.freezeSnapshot()).not.toBeNull();
    });

    it("asks for nothing when a frame is already held", async () => {
        const wait = vi.fn();
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(20);
        env.screenshot.mockClear();

        clip({ rects: [rect(100, 100)], wait });
        await flush();

        expect(wait).not.toHaveBeenCalled();
        expect(env.screenshot).not.toHaveBeenCalled();
    });

    it("keeps the held frame while the menu is on the pane, past the time it would have been dropped", async () => {
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(20);
        clip({ rects: [rect(100, 100)], wait: vi.fn() });
        await vi.advanceTimersByTimeAsync(10_000);
        expect(fz.freezeSnapshot()).not.toBeNull();
    });

    it("starts no capture when the host gave nothing to wait with", async () => {
        mount();
        clip({ rects: [rect(100, 100)] });
        await flush();
        expect(env.screenshot).not.toHaveBeenCalled();
    });

    it("drops the frame 250ms after the menu leaves the pane, and not before", async () => {
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(20);

        clip({ rects: [] });
        await vi.advanceTimersByTimeAsync(249);
        expect(fz.freezeSnapshot()).not.toBeNull();
        await vi.advanceTimersByTimeAsync(1);
        expect(fz.freezeSnapshot()).toBeNull();
    });

    it("keeps the frame when the menu comes back inside those 250ms", async () => {
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(20);
        clip({ rects: [] });
        await vi.advanceTimersByTimeAsync(100);
        clip({ rects: [rect(100, 100)], wait: vi.fn() });
        await vi.advanceTimersByTimeAsync(1000);
        expect(fz.freezeSnapshot()).not.toBeNull();
    });

    it("treats an event with no detail as a menu that left", async () => {
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(20);
        clip(undefined);
        await vi.advanceTimersByTimeAsync(250);
        expect(fz.freezeSnapshot()).toBeNull();
    });

    describe("what counts as being on the pane", () => {
        const hits = async (r: { x: number; y: number; w: number; h: number }) => {
            const wait = vi.fn();
            mount();
            clip({ rects: [r], wait });
            await flush();
            return wait.mock.calls.length === 1;
        };

        it("a menu over the pane's middle is", async () => {
            expect(await hits(rect(100, 100))).toBe(true);
        });

        it("a menu that overlaps by a pixel on each side is", async () => {
            expect(await hits({ x: 314, y: 100, w: 50, h: 50 })).toBe(true);
            cleanup();
            expect(await hits({ x: -34, y: 100, w: 50, h: 50 })).toBe(true);
            cleanup();
            expect(await hits({ x: 100, y: 224, w: 50, h: 50 })).toBe(true);
            cleanup();
            expect(await hits({ x: 100, y: -24, w: 50, h: 50 })).toBe(true);
        });

        it("a menu that only touches an edge is not", async () => {
            expect(await hits({ x: 315, y: 100, w: 50, h: 50 })).toBe(false);
            cleanup();
            expect(await hits({ x: -35, y: 100, w: 50, h: 50 })).toBe(false);
            cleanup();
            expect(await hits({ x: 100, y: 225, w: 50, h: 50 })).toBe(false);
            cleanup();
            expect(await hits({ x: 100, y: -25, w: 50, h: 50 })).toBe(false);
        });

        it("is judged on the pane's physical rectangle converted to CSS pixels, not on the raw numbers", async () => {
            // Right of the pane at a ratio of 2 (315), but left of the unconverted right edge (630).
            expect(await hits({ x: 320, y: 100, w: 50, h: 50 })).toBe(false);
        });

        it("is judged at a ratio of 1 when the window has none", async () => {
            Object.defineProperty(window, "devicePixelRatio", { value: 0, configurable: true });
            // The pane is then at (30, 50) and 600 x 400: a menu over its middle lands on it.
            expect(await hits({ x: 100, y: 100, w: 50, h: 50 })).toBe(true);
            cleanup();
            // And one to the right of 630 does not.
            expect(await hits({ x: 640, y: 100, w: 50, h: 50 })).toBe(false);
        });

        it("is never a menu over a pane with no width or no height", async () => {
            // A rectangle that straddles the pane's line.
            s.rect = { x: 30, y: 50, width: 0, height: 400 };
            expect(await hits({ x: 0, y: 100, w: 50, h: 50 })).toBe(false);
            cleanup();
            s.rect = { x: 30, y: 50, width: 600, height: 0 };
            expect(await hits({ x: 100, y: 0, w: 50, h: 50 })).toBe(false);
        });

        it("any one of several rectangles is enough", async () => {
            const wait = vi.fn();
            mount();
            clip({ rects: [rect(900, 900), rect(100, 100)], wait });
            await flush();
            expect(wait).toHaveBeenCalledTimes(1);
        });
    });
});

describe("while it is mounted", () => {
    it("stops listening, and cancels the pending drop, when the view goes", async () => {
        mount();
        prewarm();
        await vi.advanceTimersByTimeAsync(20);
        unmount();
        await vi.advanceTimersByTimeAsync(10_000);
        // The 4 seconds were cancelled with the view: nothing clears the frame, and nothing reacts any more.
        expect(fz.freezeSnapshot()).not.toBeNull();
        env.screenshot.mockClear();
        prewarm();
        clip({ rects: [{ x: 100, y: 100, w: 50, h: 50 }], wait: vi.fn() });
        await flush();
        expect(env.screenshot).not.toHaveBeenCalled();
    });

    it("does not listen before it is mounted", async () => {
        prewarm();
        await flush();
        expect(env.screenshot).not.toHaveBeenCalled();
    });
});
