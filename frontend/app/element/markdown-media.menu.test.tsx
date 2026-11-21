// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A right-click on an inline video or audio in an agent's reply gives the media viewer's items (Copy frame for a
 * video, Copy path for both), before play as well as after, in place of the pane's menu.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    shown: [] as { label: string; click?: () => void }[][],
    notes: [] as { title: string; message: string }[],
    copied: [] as string[],
}));

vi.mock("@/app/element/local-media", async () => {
    const actual = await vi.importActual<typeof import("@/app/element/local-media")>("@/app/element/local-media");
    return {
        ...actual,
        fetchMediaRange: async () => ({ blob: new Blob([new Uint8Array(8)]), total: 5_000_000 }),
        fetchMediaBlob: async () => new Blob([new Uint8Array(16)]),
    };
});
vi.mock("@/app/motion/resize-contract", () => ({ withHeightContinuity: (_el: HTMLElement, fn: () => void) => fn() }));
vi.mock("@/app/store/block-layout-actions", () => ({ createBlock: async () => "new-block" }));
vi.mock("@/app/store/global", () => ({
    openLink: () => {},
    pushNotification: (n: { title: string; message: string }) => h.notes.push(n),
}));
vi.mock("@/app/store/contextmenu", () => ({
    ContextMenuModel: { showContextMenu: (items: { label: string }[]) => h.shown.push(items) },
}));
vi.mock("@/util/clipboard", () => ({
    writeImage: async () => {},
    writeText: async (t: string) => void h.copied.push(t),
}));
vi.mock("@/app/util/editor-open", () => ({ openInEditorPaneOnScreen: async () => false }));
vi.mock("@/app/element/tip", () => ({ tip: (text: string) => ({ "data-tip": text }) }));

import { MarkdownImg } from "./markdown-media";

class FakeObserver {
    static last: FakeObserver;
    constructor(public cb: (entries: { isIntersecting: boolean }[]) => void) {
        FakeObserver.last = this;
    }
    observe() {}
    disconnect() {}
}

const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};

let host: HTMLElement;
let reachedPane = false;
/** The pane's own handler sits above the reply, and is a Solid one (delegated) like the block frame's. */
function mount(src: string) {
    host = render(() => (
        <div onContextMenu={() => (reachedPane = true)}>
            <MarkdownImg props={{ src, alt: "clip" }} media={{ baseDir: "C:/work" }} />
        </div>
    )).container;
    FakeObserver.last.cb([{ isIntersecting: true }]);
}

/** A right-click on `target`: whether the page's menu was cancelled and whether the pane's handler heard of it. */
function rightClick(target: Element): { defaultPrevented: boolean; reachedPane: boolean } {
    reachedPane = false;
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    target.dispatchEvent(event);
    return { defaultPrevented: event.defaultPrevented, reachedPane };
}

const labels = () => h.shown.at(-1)!.map((i) => i.label);

beforeEach(() => {
    h.shown.length = 0;
    h.notes.length = 0;
    h.copied.length = 0;
    vi.stubGlobal("IntersectionObserver", FakeObserver);
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => "blob:test/1", revokeObjectURL: () => {} }));
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

describe("an inline video in a reply", () => {
    it("gives Copy frame and Copy path on its overlay before play, and the pane's menu does not open", async () => {
        mount("clips/demo.mp4");
        await flush();
        const result = rightClick(host.querySelector(".am-media-stage-overlay")!);
        expect(labels()).toEqual(["Copy frame", "Copy path"]);
        expect(result).toEqual({ defaultPrevented: true, reachedPane: false });
    });

    it("gives the same items on the poster and on the play button", async () => {
        mount("clips/demo.mp4");
        await flush();
        for (const target of [host.querySelector(".am-media-poster")!, host.querySelector(".am-media-play")!]) {
            h.shown.length = 0;
            rightClick(target);
            expect(labels()).toEqual(["Copy frame", "Copy path"]);
        }
    });

    it("copies the path it was opened from", async () => {
        mount("clips/demo.mp4");
        await flush();
        rightClick(host.querySelector(".am-media-stage-overlay")!);
        h.shown.at(-1)!.find((i) => i.label === "Copy path")!.click!();
        await flush();
        expect(h.copied).toEqual([expect.stringMatching(/clips[\\/]demo\.mp4$/)]);
    });

    it("Copy frame reads the poster before play (it says the frame is not ready, not that no video is on show)", async () => {
        mount("clips/demo.mp4");
        await flush();
        rightClick(host.querySelector(".am-media-stage-overlay")!);
        h.shown.at(-1)!.find((i) => i.label === "Copy frame")!.click!();
        await flush();
        expect(h.notes.at(-1)).toMatchObject({ title: "Couldn't copy the image", message: "The video has no frame to copy yet." });
    });

    it("gives the same items on the player once it plays", async () => {
        mount("clips/demo.mp4");
        await flush();
        fireEvent.click(host.querySelector(".am-media-play")!);
        await flush();
        const player = host.querySelector("video.am-media-player")!;
        const result = rightClick(player);
        expect(labels()).toEqual(["Copy frame", "Copy path"]);
        expect(result).toEqual({ defaultPrevented: true, reachedPane: false });
        h.shown.at(-1)!.find((i) => i.label === "Copy frame")!.click!();
        await flush();
        expect(h.notes.at(-1)!.message).toBe("The video has no frame to copy yet.");
    });
});

describe("an inline audio in a reply", () => {
    it("gives Copy path only, on its card before play and on the player after", async () => {
        mount("sound/take.wav");
        await flush();
        const result = rightClick(host.querySelector(".am-media-audio-card")!);
        expect(labels()).toEqual(["Copy path"]);
        expect(result).toEqual({ defaultPrevented: true, reachedPane: false });
        fireEvent.click(host.querySelector(".am-media-play")!);
        await flush();
        h.shown.length = 0;
        rightClick(host.querySelector("audio.am-media-player")!);
        expect(labels()).toEqual(["Copy path"]);
    });
});
