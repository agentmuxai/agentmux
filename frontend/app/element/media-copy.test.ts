// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Copy for media: what each kind puts on the clipboard, what it refuses and says, and the right-click items.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    writeImage: vi.fn(async (_png: Blob) => {}),
    writeText: vi.fn(async (_t: string) => {}),
    fetchMediaBlob: vi.fn(),
    notes: [] as { title: string; message: string; type: string }[],
    shown: [] as unknown[],
}));

vi.mock("@/util/clipboard", () => ({ writeImage: h.writeImage, writeText: h.writeText }));
vi.mock("@/app/store/global", () => ({ pushNotification: (n: { title: string; message: string; type: string }) => h.notes.push(n) }));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: (items: unknown) => h.shown.push(items) } }));
vi.mock("./local-media", async (orig) => ({ ...(await orig<typeof import("./local-media")>()), fetchMediaBlob: h.fetchMediaBlob }));

import { MediaTooLargeError } from "./local-media";
import {
    copyMediaImage,
    copyMediaWithNote,
    copyNote,
    IMAGE_COPY_MAX_BYTES,
    IMAGE_COPY_MAX_PIXELS,
    mediaCopyMenuItems,
    showMediaCopyMenu,
    sourceFromUrl,
    type MediaSource,
} from "./media-copy";

const blobOf = (type: string, size = 8) => new Blob([new Uint8Array(size)], { type });

// What the browser would decode, and the canvas it would draw to: the page under test is jsdom.
const decoded = { w: 40, h: 20 };
const drawn: { w: number; h: number }[] = [];
beforeEach(() => {
    h.writeImage.mockClear();
    h.writeText.mockClear();
    h.fetchMediaBlob.mockReset();
    h.notes.length = 0;
    h.shown.length = 0;
    drawn.length = 0;
    decoded.w = 40;
    decoded.h = 20;
    URL.createObjectURL = () => "blob:x";
    URL.revokeObjectURL = () => {};
    vi.stubGlobal(
        "Image",
        class {
            src = "";
            get naturalWidth() {
                return decoded.w;
            }
            get naturalHeight() {
                return decoded.h;
            }
            decode = async () => {};
        }
    );
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement) {
        return { drawImage: () => drawn.push({ w: this.width, h: this.height }) } as unknown as CanvasRenderingContext2D;
    } as unknown as typeof HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.toBlob = function (cb: BlobCallback) {
        cb(new Blob([new Uint8Array(100)], { type: "image/png" }));
    };
});
afterEach(() => vi.unstubAllGlobals());

const image = (over: Partial<MediaSource> = {}): MediaSource => ({ kind: "image", mime: "image/png", path: "C:/a/b.png", ...over });

describe("copying an image", () => {
    it("a PNG goes on the clipboard as it is, with no redraw", async () => {
        const png = blobOf("image/png");
        h.fetchMediaBlob.mockResolvedValue(png);
        expect(await copyMediaImage(image())).toEqual({ ok: true, what: "image" });
        expect(h.writeImage).toHaveBeenCalledWith(png);
        expect(drawn).toEqual([]);
    });

    it.each([
        ["image/jpeg", "image"],
        ["image/svg+xml", "image"],
        ["image/avif", "image"],
        ["image/gif", "first frame"],
        ["image/webp", "first frame"],
        ["image/apng", "first frame"],
    ])("%s is drawn and copied as a PNG, and the note says %s", async (mime, what) => {
        h.fetchMediaBlob.mockResolvedValue(blobOf(mime));
        expect(await copyMediaImage(image({ mime }))).toEqual({ ok: true, what });
        expect(drawn).toEqual([{ w: 40, h: 20 }]);
        expect(h.writeImage.mock.calls[0][0].type).toBe("image/png");
    });

    it("an SVG with no size of its own is drawn 1024 wide", async () => {
        decoded.w = 0;
        decoded.h = 0;
        h.fetchMediaBlob.mockResolvedValue(blobOf("image/svg+xml"));
        expect((await copyMediaImage(image({ mime: "image/svg+xml" }))).ok).toBe(true);
        expect(drawn).toEqual([{ w: 1024, h: 1024 }]);
    });

    it("the bytes the caller holds are used, not the path (a file over SSH, a dropped file, a reply's image)", async () => {
        const mine = blobOf("image/png");
        expect((await copyMediaImage(image({ path: undefined, bytes: async () => mine }))).ok).toBe(true);
        expect(h.fetchMediaBlob).not.toHaveBeenCalled();
        expect(h.writeImage).toHaveBeenCalledWith(mine);
    });

    it("an image with neither bytes nor a path says so", async () => {
        expect(await copyMediaImage(image({ path: undefined }))).toEqual({ ok: false, reason: "there is no file to read" });
    });
});

describe("what is too large", () => {
    it("a picture over the pixel limit is refused before it is drawn", async () => {
        decoded.w = 9000;
        decoded.h = 9000;
        expect(9000 * 9000).toBeGreaterThan(IMAGE_COPY_MAX_PIXELS);
        h.fetchMediaBlob.mockResolvedValue(blobOf("image/jpeg"));
        const r = await copyMediaImage(image({ mime: "image/jpeg" }));
        expect(r).toMatchObject({ ok: false });
        expect((r as { reason: string }).reason).toMatch(/9000 x 9000 px/);
        expect(drawn).toEqual([]);
        expect(h.writeImage).not.toHaveBeenCalled();
    });

    it("a PNG over the byte limit is refused and says the limit", async () => {
        h.fetchMediaBlob.mockResolvedValue(blobOf("image/png", IMAGE_COPY_MAX_BYTES + 1));
        const r = await copyMediaImage(image());
        expect(r).toMatchObject({ ok: false });
        expect((r as { reason: string }).reason).toMatch(/25 MB/);
        expect(h.writeImage).not.toHaveBeenCalled();
    });

    it("a file over the fetch limit says its size", async () => {
        h.fetchMediaBlob.mockRejectedValue(new MediaTooLargeError(40 * 1024 * 1024));
        const r = await copyMediaImage(image());
        expect((r as { reason: string }).reason).toBe("the file is 40 MB, more than the 25 MB this copies");
    });
});

describe("failing", () => {
    it("a refusal by the clipboard is the reason, and nothing throws", async () => {
        h.fetchMediaBlob.mockResolvedValue(blobOf("image/png"));
        h.writeImage.mockRejectedValueOnce(new Error("the clipboard is in use by another program; try again"));
        expect(await copyMediaImage(image())).toEqual({ ok: false, reason: "the clipboard is in use by another program; try again" });
    });

    it("a file that can't be fetched is the reason", async () => {
        h.fetchMediaBlob.mockRejectedValue(new Error("404 Not Found"));
        expect(await copyMediaImage(image())).toEqual({ ok: false, reason: "404 Not Found" });
    });

    it("audio and a PDF have no picture", async () => {
        for (const kind of ["audio", "pdf"] as const) {
            expect(await copyMediaImage({ kind, mime: "", path: "C:/a" })).toMatchObject({ ok: false });
        }
    });
});

describe("a video's frame", () => {
    const video = (w: number, h2: number): MediaSource => {
        const el = Object.assign(document.createElement("video"), {});
        Object.defineProperty(el, "videoWidth", { value: w });
        Object.defineProperty(el, "videoHeight", { value: h2 });
        return { kind: "video", mime: "video/mp4", path: "C:/a/v.mp4", element: () => el };
    };

    it("copies the frame on show, at the video's size", async () => {
        expect(await copyMediaImage(video(1280, 720))).toEqual({ ok: true, what: "frame" });
        expect(drawn).toEqual([{ w: 1280, h: 720 }]);
        expect(h.fetchMediaBlob).not.toHaveBeenCalled();
    });

    it("before the first frame, or with no video on show, says so", async () => {
        expect(await copyMediaImage(video(0, 0))).toEqual({ ok: false, reason: "the video has no frame to copy yet" });
        expect(await copyMediaImage({ kind: "video", mime: "", element: () => null })).toEqual({ ok: false, reason: "the video is not on show" });
    });
});

describe("the note", () => {
    it("says what was copied, or why not", () => {
        expect(copyNote({ ok: true, what: "image" })).toEqual({ title: "Copied the image", message: "", type: "info" });
        expect(copyNote({ ok: true, what: "first frame" }).title).toBe("Copied the first frame");
        expect(copyNote({ ok: true, what: "frame" }).title).toBe("Copied the frame");
        expect(copyNote({ ok: false, reason: "the video has no frame to copy yet" })).toEqual({
            title: "Couldn't copy the image",
            message: "The video has no frame to copy yet.",
            type: "error",
        });
    });

    it("is shown after the copy", async () => {
        h.fetchMediaBlob.mockResolvedValue(blobOf("image/png"));
        await copyMediaWithNote(image());
        expect(h.notes.map((n) => n.title)).toEqual(["Copied the image"]);
        h.fetchMediaBlob.mockRejectedValue(new Error("gone"));
        await copyMediaWithNote(image());
        expect(h.notes.at(-1)).toMatchObject({ title: "Couldn't copy the image", type: "error" });
    });
});

describe("the right-click items", () => {
    const labels = (s: MediaSource, selection = "") => mediaCopyMenuItems(s, selection).map((i) => i.label);

    it.each([
        ["an image file", image(), "", ["Copy", "Copy path"]],
        ["a dropped image (no path)", image({ path: undefined, bytes: async () => blobOf("image/png") }), "", ["Copy"]],
        ["a video", { kind: "video", mime: "video/mp4", path: "C:/v.mp4" } as MediaSource, "", ["Copy frame", "Copy path"]],
        ["audio", { kind: "audio", mime: "audio/wav", path: "C:/a.wav" } as MediaSource, "", ["Copy path"]],
        ["a PDF with nothing selected", { kind: "pdf", mime: "application/pdf", path: "C:/a.pdf" } as MediaSource, "", ["Copy path"]],
        ["a PDF with text selected", { kind: "pdf", mime: "application/pdf", path: "C:/a.pdf" } as MediaSource, "some text", ["Copy", "Copy path"]],
    ])("%s offers %j", (_name, source, selection, expected) => {
        expect(labels(source, selection)).toEqual(expected);
    });

    it("Copy copies the picture and Copy path copies the path, each with its note", async () => {
        h.fetchMediaBlob.mockResolvedValue(blobOf("image/png"));
        const [copy, copyPath] = mediaCopyMenuItems(image());
        copy.click!();
        copyPath.click!();
        await vi.waitFor(() => expect(h.notes.map((n) => n.title).sort()).toEqual(["Copied the image", "Copied the path"]));
        expect(h.writeText).toHaveBeenCalledWith("C:/a/b.png");
    });

    it("a PDF's selection copies as text", () => {
        const [copy] = mediaCopyMenuItems({ kind: "pdf", mime: "", path: "C:/a.pdf" }, "some text");
        copy.click!();
        expect(h.writeText).toHaveBeenCalledWith("some text");
    });

    it("an image on screen from a URL opens the same menu, over the page's own", () => {
        const e = { preventDefault: vi.fn(), stopPropagation: vi.fn() } as unknown as MouseEvent;
        showMediaCopyMenu(e, sourceFromUrl("blob:x", { path: "C:/a/b.png" }));
        expect(e.preventDefault).toHaveBeenCalled();
        expect((h.shown[0] as { label: string }[]).map((i) => i.label)).toEqual(["Copy", "Copy path"]);
    });
});
