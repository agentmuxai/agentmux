// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The audio and video viewer's extra controls: loop, speed and the frame step,
// from the bar and from the keys.
// docs/reports/REPORT_MEDIA_VIEWER_ZOOM_AND_FEATURES_2026_10_09.md §4 row 7.

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AvViewer } from "./av-viewer";

// A file from disk has no URL until its bytes arrive, so the media element mounts later.
const mockBytes = vi.hoisted(() => ({ initial: "blob:media" as string | null, setUrl: (_: string | null) => {} }));

vi.mock("../file-bytes", () => ({
    createFileBytes: () => {
        const [url, setUrl] = createSignal<string | null>(mockBytes.initial);
        mockBytes.setUrl = setUrl;
        const [error, setError] = createSignal("");
        return { url, error, setError };
    },
}));

function mount(mime: string, path = "C:/clips/a.mp4") {
    const { container } = render(() => (
        <AvViewer path={path} mime={mime} revision={0} bar={{ viewer: "media", options: ["media"], onReopenWith: () => {} }} />
    ));
    const el = container.querySelector<HTMLMediaElement>("video, audio")!;
    fireEvent.loadedData(el);
    return { container, el };
}

const button = (c: HTMLElement, label: RegExp) =>
    Array.from(c.querySelectorAll<HTMLButtonElement>("button")).find((b) => label.test(b.getAttribute("aria-label") ?? b.title))!;

describe("the audio and video viewer's controls", () => {
    afterEach(() => {
        cleanup();
        mockBytes.initial = "blob:media";
    });

    it("drives an element that mounts after the file's bytes arrive", () => {
        mockBytes.initial = null;
        const { container } = render(() => (
            <AvViewer path="C:/clips/a.mp4" mime="video/mp4" revision={0} bar={{ viewer: "media", options: ["media"], onReopenWith: () => {} }} />
        ));
        expect(container.querySelector("video")).toBeNull();
        mockBytes.setUrl("blob:media");
        const el = container.querySelector<HTMLMediaElement>("video")!;
        fireEvent.loadedData(el);
        fireEvent.click(button(container, /^Loop/));
        expect(el.loop).toBe(true);
        fireEvent.keyDown(el, { key: ">" });
        expect(el.playbackRate).toBe(1.25);
    });

    it("starts at normal speed, without a loop", () => {
        const { el, container } = mount("video/mp4");
        expect(el.playbackRate).toBe(1);
        expect(el.loop).toBe(false);
        expect(container.textContent).toContain("1×");
    });

    it("loops from the button and from L", () => {
        const { el, container } = mount("video/mp4");
        fireEvent.click(button(container, /^Loop/));
        expect(el.loop).toBe(true);
        fireEvent.keyDown(el, { key: "l" });
        expect(el.loop).toBe(false);
    });

    it("changes speed in quarter steps from the keys and the buttons", () => {
        const { el, container } = mount("video/mp4");
        fireEvent.keyDown(el, { key: ">" });
        expect(el.playbackRate).toBe(1.25);
        fireEvent.click(button(container, /^Faster/));
        expect(el.playbackRate).toBe(1.5);
        fireEvent.keyDown(el, { key: "<" });
        fireEvent.keyDown(el, { key: "<" });
        expect(el.playbackRate).toBe(1);
        fireEvent.keyDown(el, { key: "<" });
        expect(el.playbackRate).toBe(0.75);
        expect(container.textContent).toContain("0.75×");
        fireEvent.click(button(container, /Normal speed/));
        expect(el.playbackRate).toBe(1);
    });

    it("pauses and steps a frame with . and ,", () => {
        const { el } = mount("video/mp4");
        const pause = vi.spyOn(el, "pause").mockImplementation(() => {});
        Object.defineProperty(el, "duration", { configurable: true, value: 10 });
        el.currentTime = 2;
        fireEvent.keyDown(el, { key: "." });
        expect(pause).toHaveBeenCalled();
        expect(el.currentTime).toBeCloseTo(2 + 1 / 30);
        fireEvent.keyDown(el, { key: "," });
        expect(el.currentTime).toBeCloseTo(2);
    });

    it("steps by the shortest gap between shown frames, not by a gap that spans several", () => {
        let onFrame: VideoFrameRequestCallback = () => {};
        const proto = HTMLVideoElement.prototype as unknown as Record<string, unknown>;
        proto.requestVideoFrameCallback = (cb: VideoFrameRequestCallback) => {
            onFrame = cb;
            return 1;
        };
        proto.cancelVideoFrameCallback = () => {};
        try {
            const { el } = mount("video/mp4");
            // 25 fps clip at 4× speed: shown frames are 0.04 or 0.08 s apart.
            for (const t of [1, 1.08, 1.12, 1.2]) onFrame(0, { mediaTime: t } as VideoFrameCallbackMetadata);
            vi.spyOn(el, "pause").mockImplementation(() => {});
            Object.defineProperty(el, "duration", { configurable: true, value: 10 });
            el.currentTime = 2;
            fireEvent.keyDown(el, { key: "." });
            expect(el.currentTime).toBeCloseTo(2.04);
        } finally {
            delete proto.requestVideoFrameCallback;
            delete proto.cancelVideoFrameCallback;
        }
    });

    it("has no frame step for audio but keeps loop and speed", () => {
        const { el, container } = mount("audio/wav", "C:/clips/a.wav");
        expect(button(container, /Back one frame/)).toBeUndefined();
        el.currentTime = 2;
        fireEvent.keyDown(el, { key: "." });
        expect(el.currentTime).toBe(2);
        fireEvent.keyDown(el, { key: ">" });
        expect(el.playbackRate).toBe(1.25);
    });

    it("takes the shortcuts from a focused toolbar button too", () => {
        const { el, container } = mount("video/mp4");
        fireEvent.keyDown(button(container, /^Loop/), { key: ">" });
        expect(el.playbackRate).toBe(1.25);
    });

    it("leaves the app's shortcuts to the app", () => {
        const { el } = mount("video/mp4");
        const e = new KeyboardEvent("keydown", { key: "l", ctrlKey: true, bubbles: true, cancelable: true });
        el.dispatchEvent(e);
        expect(e.defaultPrevented).toBe(false);
        expect(el.loop).toBe(false);
    });
});
