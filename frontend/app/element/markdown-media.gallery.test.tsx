// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a click on an image or a video's enlarge button hands the agent pane's preview: every image and video of the
 * message, in reading order, with the place of the one clicked, and for a playing video where it was (so the enlarged
 * one carries on from there and the inline one stops).
 */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ createBlock: vi.fn(async () => "new-block") }));

vi.mock("@/app/element/local-media", async () => {
    const actual = await vi.importActual<typeof import("@/app/element/local-media")>("@/app/element/local-media");
    return {
        ...actual,
        fetchMediaBlob: async () => new Blob([new Uint8Array(8)]),
        fetchMediaRange: async () => ({ blob: new Blob([new Uint8Array(8)]), total: 4_000_000 }),
    };
});
vi.mock("@/app/store/block-layout-actions", () => ({ createBlock: h.createBlock }));
vi.mock("@/app/store/global", () => ({ openLink: () => {} }));
vi.mock("@/app/util/editor-open", () => ({ openInEditorPaneOnScreen: async () => false }));
vi.mock("@/app/element/tip", () => ({ tip: (text: string) => ({ "data-tip": text }) }));

import { handBackVideo, MarkdownImg, type MediaGallery } from "./markdown-media";

const BASE = "C:/work";
let n = 0;

beforeEach(() => {
    n = 0;
    h.createBlock.mockClear();
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => `blob:g/${++n}`, revokeObjectURL: () => {} }));
    // jsdom has no media playback: only what the pane asks of the player is recorded.
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(function (this: HTMLMediaElement) {
        Object.defineProperty(this, "paused", { value: true, configurable: true });
    });
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

/** A message of the given media, as the agent pane draws it: each through `MarkdownImg`, in one markdown block. */
function message(files: string[], openGallery: (g: MediaGallery) => void) {
    return render(() => (
        <div class="agent-markdown-block">
            {files.map((src) => (
                <MarkdownImg props={{ src, alt: "" } as never} media={{ baseDir: BASE, openGallery }} />
            ))}
        </div>
    ));
}

describe("a click on an image of a message", () => {
    it("opens the preview even when it is the message's only image, not the Editor", async () => {
        const open = vi.fn();
        const { container } = message(["shots/a.png"], open);
        await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
        fireEvent.click(container.querySelector("img")!);
        expect(open).toHaveBeenCalledTimes(1);
        expect(open.mock.calls[0][0]).toEqual({
            items: [{ path: "C:/work/shots/a.png", name: "a.png", alt: "", kind: "image" }],
            index: 0,
        });
        expect(h.createBlock).not.toHaveBeenCalled();
    });

    it("hands over the message's images and videos in reading order, and the place of the one clicked", async () => {
        const open = vi.fn();
        const { container } = message(["a.png", "clip.mp4", "b.png", "voice.wav"], open);
        await waitFor(() => expect(container.querySelectorAll("img")).toHaveLength(2));
        fireEvent.click(container.querySelectorAll("img")[1]);
        const gallery = open.mock.calls[0][0] as MediaGallery;
        expect(gallery.items.map((i) => [i.name, i.kind])).toEqual([
            ["a.png", "image"],
            ["clip.mp4", "video"],
            ["b.png", "image"],
        ]);
        expect(gallery.index).toBe(2);
        expect(gallery.start).toBeUndefined();
    });
});

describe("a video's enlarge button", () => {
    const ready = (container: HTMLElement) => waitFor(() => expect(container.querySelector(".am-media-enlarge")).not.toBeNull());
    const play = async (container: HTMLElement) => {
        await ready(container);
        await waitFor(() => expect(container.querySelector<HTMLButtonElement>(".am-media-play")!.disabled).toBe(false));
        fireEvent.click(container.querySelector(".am-media-play")!);
        await waitFor(() => expect(container.querySelector("video.am-media-player")).not.toBeNull());
        return container.querySelector<HTMLVideoElement>("video.am-media-player")!;
    };

    it("is only offered where the pane can show the preview", async () => {
        const { container } = render(() => (
            <div class="agent-markdown-block">
                <MarkdownImg props={{ src: "clip.mp4", alt: "" } as never} media={{ baseDir: BASE }} />
            </div>
        ));
        await waitFor(() => expect(container.querySelector(".am-media-play")).not.toBeNull());
        expect(container.querySelector(".am-media-enlarge")).toBeNull();
    });

    it("before the video was played, starts the enlarged one at the beginning, muted and waiting", async () => {
        const open = vi.fn();
        const { container } = message(["clip.mp4"], open);
        await ready(container);
        fireEvent.click(container.querySelector(".am-media-enlarge")!);
        const gallery = open.mock.calls[0][0] as MediaGallery;
        expect(gallery.index).toBe(0);
        expect(gallery.start).toEqual({ path: "C:/work/clip.mp4", time: 0, muted: true, paused: true });
    });

    it("from a playing video hands over its time and sound, and stops it so the sound never plays twice", async () => {
        const open = vi.fn();
        const { container } = message(["clip.mp4"], open);
        const video = await play(container);
        Object.defineProperty(video, "currentTime", { value: 12.5, writable: true, configurable: true });
        Object.defineProperty(video, "paused", { value: false, configurable: true });
        video.muted = false;
        fireEvent.click(container.querySelector(".am-media-enlarge")!);
        expect((open.mock.calls[0][0] as MediaGallery).start).toEqual({
            path: "C:/work/clip.mp4",
            time: 12.5,
            muted: false,
            paused: false,
        });
        expect(HTMLMediaElement.prototype.pause).toHaveBeenCalled();
        expect(video.paused).toBe(true);
    });

    it("takes the position back when the enlarged player closes: the inline one stays paused at it, sound as set", async () => {
        const { container } = message(["clip.mp4"], vi.fn());
        const video = await play(container);
        Object.defineProperty(video, "currentTime", { value: 0, writable: true, configurable: true });
        handBackVideo({ path: "C:/work/clip.mp4", time: 31, muted: false });
        expect(video.currentTime).toBe(31);
        expect(video.muted).toBe(false);
        expect(video.paused).toBe(true);
    });

    it("hands back to nothing once the inline player is gone", async () => {
        const { container, unmount } = message(["clip.mp4"], vi.fn());
        await play(container);
        unmount();
        expect(() => handBackVideo({ path: "C:/work/clip.mp4", time: 3, muted: true })).not.toThrow();
    });
});
