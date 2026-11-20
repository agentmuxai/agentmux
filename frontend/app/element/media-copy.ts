// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Copy an image, or what stands for other media, to the system clipboard: the one function behind the Editor's
 * image viewer, an image in an agent's reply and the media viewer, so they behave alike.
 *
 * What each kind puts on the clipboard (the menu says it, and a note after the copy says what was copied):
 *   - an image (PNG, JPEG, WebP, AVIF, BMP, ICO, SVG): the image, as a PNG. An SVG is drawn at its own size (or
 *     1024 px wide when it has none); its source stays copyable from the text viewer.
 *   - a GIF, WebP or APNG: its first frame; a clipboard takes one picture.
 *   - a video: "Copy frame", the frame on show now.
 *   - audio and a PDF: "Copy path". A PDF's selected text copies as text, which the pane's own Copy does.
 *
 * Over IMAGE_COPY_MAX_BYTES (as a PNG) or IMAGE_COPY_MAX_PIXELS, nothing is copied and the note says so, with
 * Copy path as the way on. A file over SSH is copied from the bytes the viewer already fetched (`blob`), never
 * from a local path.
 */

import { ContextMenuModel } from "@/app/store/contextmenu";
import { pushNotification } from "@/app/store/global";
import { MENU_ICON } from "@/app/util/menu-icons";
import { writeImage, writeText } from "@/util/clipboard";
import { fetchMediaBlob, INLINE_IMAGE_MAX_BYTES, MediaTooLargeError } from "./local-media";

/** The most a copied image may weigh as a PNG: the host takes no more (crates/cef clipboard_image.rs). */
export const IMAGE_COPY_MAX_BYTES = 25 * 1024 * 1024;
/** The most pixels drawn to copy an image: a canvas past it fails in the browser, or takes the window with it. */
export const IMAGE_COPY_MAX_PIXELS = 64 * 1024 * 1024;
/** An SVG with no size of its own is drawn this wide. */
const SVG_FALLBACK_SIDE = 1024;

export type MediaKind = "image" | "video" | "audio" | "pdf";

export interface MediaSource {
    kind: MediaKind;
    /** The type the viewer was opened with ("image/gif"), or "". */
    mime: string;
    /** The file's path, for Copy path; empty for dropped bytes and an image that is no file. */
    path?: string;
    /** The image's bytes when the caller holds them (a reply's base64 image, dropped bytes). Else the path is fetched. */
    bytes?: () => Promise<Blob>;
    /** The element showing it, for a video frame and a decoded image. */
    element?: () => HTMLImageElement | HTMLVideoElement | null | undefined;
}

export type CopyResult =
    | { ok: true; what: "image" | "first frame" | "frame" }
    | { ok: false; reason: string };

/** A raster type that can hold more than one frame: only the first is copied. */
const MULTI_FRAME = /^image\/(gif|webp|apng)$/i;

const fail = (reason: string): CopyResult => ({ ok: false, reason });

/** Whether `source` has a picture to copy (an image, or a video's frame). */
export function copiesAsImage(source: MediaSource): boolean {
    return source.kind === "image" || source.kind === "video";
}

async function bytesOf(source: MediaSource): Promise<Blob> {
    if (source.bytes) return source.bytes();
    if (!source.path) throw new Error("there is no file to read");
    return fetchMediaBlob(source.path, { maxBytes: INLINE_IMAGE_MAX_BYTES });
}

/** An image the browser can draw, from `blob` (an SVG needs its type to draw). */
async function decode(blob: Blob, mime: string): Promise<HTMLImageElement> {
    const typed = mime && blob.type !== mime ? new Blob([blob], { type: mime }) : blob;
    const url = URL.createObjectURL(typed);
    try {
        const img = new Image();
        img.src = url;
        await img.decode();
        return img;
    } finally {
        URL.revokeObjectURL(url);
    }
}

function toPng(canvas: HTMLCanvasElement): Promise<Blob> {
    return new Promise((resolve, reject) =>
        canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("the browser could not make a PNG of it"))), "image/png")
    );
}

/** Draw `picture` at `w` x `h` and return its PNG, or why not: a size the browser can't take is refused. */
async function draw(picture: CanvasImageSource, w: number, h: number): Promise<Blob | string> {
    if (w < 1 || h < 1) return "it has no size to draw";
    if (w * h > IMAGE_COPY_MAX_PIXELS) {
        return `it is ${w} x ${h} px, more than the ${Math.floor(IMAGE_COPY_MAX_PIXELS / 1e6)} million this copies`;
    }
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return "the browser gave no canvas to draw it on";
    ctx.drawImage(picture, 0, 0, w, h);
    return toPng(canvas);
}

/** The PNG for an image: its own bytes when they already are one, else drawn from the decoded picture. `firstFrame`: it can hold more than one frame. */
async function pngOfImage(source: MediaSource): Promise<{ png: Blob | string; firstFrame: boolean }> {
    const blob = await bytesOf(source);
    const mime = source.mime || blob.type;
    const firstFrame = MULTI_FRAME.test(mime);
    if (/^image\/png$/i.test(blob.type) && !firstFrame) return { png: blob, firstFrame };
    const img = await decode(blob, mime);
    const w = img.naturalWidth || (/svg/i.test(mime) ? SVG_FALLBACK_SIDE : 0);
    const h = img.naturalHeight || (/svg/i.test(mime) ? SVG_FALLBACK_SIDE : 0);
    return { png: await draw(img, w, h), firstFrame };
}

function pngOfFrame(source: MediaSource): Promise<Blob | string> | string {
    const el = source.element?.();
    if (!(el instanceof HTMLVideoElement)) return "the video is not on show";
    if (!el.videoWidth) return "the video has no frame to copy yet";
    return draw(el, el.videoWidth, el.videoHeight);
}

function describe(e: unknown): string {
    if (e instanceof MediaTooLargeError) return `the file is ${Math.round(e.size / (1024 * 1024))} MB, more than the ${INLINE_IMAGE_MAX_BYTES / (1024 * 1024)} MB this copies`;
    const m = e instanceof Error ? e.message : String(e);
    return m || "unknown error";
}

/** Put the picture of `source` on the clipboard as an image. Never throws: the outcome says what happened. */
export async function copyMediaImage(source: MediaSource): Promise<CopyResult> {
    if (!copiesAsImage(source)) return fail("only an image or a video frame can be copied as a picture");
    try {
        const video = source.kind === "video";
        const { png, firstFrame } = video ? { png: await pngOfFrame(source), firstFrame: false } : await pngOfImage(source);
        if (typeof png === "string") return fail(png);
        if (png.size > IMAGE_COPY_MAX_BYTES) {
            return fail(`as a PNG it is ${Math.round(png.size / (1024 * 1024))} MB, more than the ${IMAGE_COPY_MAX_BYTES / (1024 * 1024)} MB this copies`);
        }
        await writeImage(png);
        return { ok: true, what: video ? "frame" : firstFrame ? "first frame" : "image" };
    } catch (e) {
        return fail(describe(e));
    }
}

/** What the note says after a copy: what went on the clipboard, or why nothing did. */
export function copyNote(result: CopyResult): { title: string; message: string; type: "info" | "error" } {
    return "reason" in result
        ? { title: "Couldn't copy the image", message: `${result.reason[0].toUpperCase()}${result.reason.slice(1)}.`, type: "error" }
        : { title: `Copied the ${result.what}`, message: "", type: "info" };
}

function note(result: CopyResult): void {
    const { title, message, type } = copyNote(result);
    pushNotification({
        icon: result.ok ? "fa-circle-check" : "fa-triangle-exclamation",
        title,
        message,
        timestamp: new Date().toISOString(),
        type,
        expiration: Date.now() + (result.ok ? 2500 : 8000),
    });
}

/** Copy `source`'s picture and say how it went (a short "Copied" note, or why not). */
export async function copyMediaWithNote(source: MediaSource): Promise<CopyResult> {
    const result = await copyMediaImage(source);
    note(result);
    return result;
}

/** Copy the file's path and say so. */
export async function copyPathWithNote(path: string): Promise<void> {
    try {
        await writeText(path);
        pushNotification({ icon: "fa-circle-check", title: "Copied the path", message: "", timestamp: new Date().toISOString(), type: "info", expiration: Date.now() + 2500 });
    } catch (e) {
        pushNotification({ icon: "fa-triangle-exclamation", title: "Couldn't copy the path", message: describe(e), timestamp: new Date().toISOString(), type: "error", expiration: Date.now() + 8000 });
    }
}

/** A source for an image or video already on screen from a `blob:` or `data:` URL (what an element's `src` holds). */
export function sourceFromUrl(url: string, opts: { kind?: MediaKind; mime?: string; path?: string; element?: MediaSource["element"] } = {}): MediaSource {
    return {
        kind: opts.kind ?? "image",
        mime: opts.mime ?? "",
        path: opts.path,
        element: opts.element,
        bytes: async () => (await fetch(url)).blob(),
    };
}

/** The right-click menu of media on screen: its Copy and Copy path, in place of the page's. */
export function showMediaCopyMenu(e: MouseEvent, source: MediaSource): void {
    e.preventDefault();
    e.stopPropagation();
    ContextMenuModel.showContextMenu(mediaCopyMenuItems(source), e);
}

/** The label of the picture item for a kind. */
export function copyLabel(kind: MediaKind): string {
    return kind === "video" ? "Copy frame" : "Copy";
}

/**
 * The right-click items for media: its picture (an image, a video frame) and its path (when it is a file). Empty for
 * a PDF's picture, and for audio there is only the path. `selection` is the text selected in the pane: a PDF's text
 * copies as text, as it does in any pane, so a PDF with a selection keeps the plain Copy.
 */
export function mediaCopyMenuItems(source: MediaSource, selection = ""): ContextMenuItem[] {
    const items: ContextMenuItem[] = [];
    if (copiesAsImage(source)) {
        items.push({ label: copyLabel(source.kind), icon: MENU_ICON.copy, click: () => void copyMediaWithNote(source) });
    } else if (selection) {
        items.push({ label: "Copy", icon: MENU_ICON.copy, click: () => void writeText(selection).catch(() => {}) });
    }
    if (source.path) {
        items.push({ label: "Copy path", icon: MENU_ICON.copy, click: () => void copyPathWithNote(source.path!) });
    }
    return items;
}
