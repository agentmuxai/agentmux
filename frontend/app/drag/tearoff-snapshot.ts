// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A picture of a pane, so its floating window can show the pane the moment
 * the window appears instead of the startup splash, until its live content
 * reveals. SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md phase 3.1.
 *
 * Taken on pointerdown on the pane's drag handle, before the drag starts: a
 * dragged pane is drawn blurred (tilelayout.scss `.dragging`), and a picture
 * taken at release would add its capture time to the tear-off. The browser
 * pane's drag catcher prewarms the same way (use-drag-snapshot.ts).
 *
 * The capture is the window's whole viewport, cropped here to the pane. A
 * clipped CDP screenshot (the block-scoped `browserPanes.screenshot`) makes
 * Chromium briefly change the visible page's viewport to render the clip,
 * which flickered the window as the drag began.
 *
 * A native browser pane is the exception: its page is a separate CEF browser
 * drawn above the DOM placeholder, so the window's capture shows only the
 * placeholder. It keeps its own screenshot, of its own whole page (no clip,
 * so no flicker).
 */

import { getApi } from "@/app/store/global";
import { Logger } from "@/util/logger";

/** A picture older than this is stale: the pane may have changed since. */
const MAX_AGE_MS = 10_000;

/** How long a tear-off waits for a capture still in flight. */
const TAKE_BUDGET_MS = 60;

/**
 * The largest picture (base64 chars) passed along. It travels in the
 * open_floating_pane_window request, whose body the host caps at 2 MiB
 * (axum's default); a bigger one would fail the tear-off itself.
 */
export const MAX_SNAPSHOT_CHARS = 1_000_000;

let held: { blockId: string; at: number; picture: Promise<string | null> } | null = null;

export interface CssRect {
    left: number;
    top: number;
    width: number;
    height: number;
}

/**
 * The pane's rect in the viewport image's pixels: CSS px scaled by the
 * image's own width over the viewport's (that covers the device pixel ratio
 * and the app zoom), clamped to the image. Null if nothing is left.
 */
export function cropRectInImage(
    rect: CssRect,
    viewportCssWidth: number,
    image: { width: number; height: number }
): { x: number; y: number; w: number; h: number } | null {
    if (viewportCssWidth <= 0) return null;
    const scale = image.width / viewportCssWidth;
    const x = Math.max(0, Math.round(rect.left * scale));
    const y = Math.max(0, Math.round(rect.top * scale));
    const w = Math.min(image.width, Math.round((rect.left + rect.width) * scale)) - x;
    const h = Math.min(image.height, Math.round((rect.top + rect.height) * scale)) - y;
    return w > 0 && h > 0 ? { x, y, w, h } : null;
}

function base64ToBlob(b64: string, type: string): Blob {
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    return new Blob([bytes], { type });
}

async function blobToBase64(blob: Blob): Promise<string> {
    const dataUrl = await new Promise<string>((resolve, reject) => {
        const r = new FileReader();
        r.onload = () => resolve(r.result as string);
        r.onerror = () => reject(r.error);
        r.readAsDataURL(blob);
    });
    return dataUrl.slice(dataUrl.indexOf(",") + 1);
}

/** The window's viewport, cropped to `rect`, as a base64 JPEG. */
async function capturePane(rect: CssRect): Promise<string | null> {
    const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
    const { jpeg_base64 } = await getApi().windows.captureViewport(label, 80);
    const full = await createImageBitmap(base64ToBlob(jpeg_base64, "image/jpeg"));
    try {
        const crop = cropRectInImage(rect, window.innerWidth, full);
        if (!crop) return null;
        const canvas = new OffscreenCanvas(crop.w, crop.h);
        canvas.getContext("2d")?.drawImage(full, crop.x, crop.y, crop.w, crop.h, 0, 0, crop.w, crop.h);
        return await blobToBase64(await canvas.convertToBlob({ type: "image/jpeg", quality: 0.8 }));
    } finally {
        full.close();
    }
}

/** Start capturing `blockId` (a base64 JPEG), replacing any earlier picture. */
export function prewarmTearOffSnapshot(blockId: string): void {
    const at = Date.now();
    // The rect now, before the drag restyles or moves anything.
    const el = document.querySelector(`[data-blockid="${CSS.escape(blockId)}"]`);
    if (!el) {
        held = null;
        return;
    }
    const r = el.getBoundingClientRect();
    const capture = el.querySelector(".browser-placeholder")
        ? getApi()
              .browserPanes.screenshot(blockId, { format: "jpeg", quality: 80 })
              .then((shot) => shot?.png_base64 || null)
        : capturePane({ left: r.left, top: r.top, width: r.width, height: r.height });
    const picture = capture.then(
        (b64) => {
            Logger.debug("dnd:cross", "tear-off snapshot captured", { blockId, ms: Date.now() - at });
            return b64;
        },
        (e) => {
            Logger.debug("dnd:cross", "tear-off snapshot failed", { blockId, error: String(e) });
            return null;
        }
    );
    held = { blockId, at, picture };
}

/**
 * The picture of `blockId` taken for this drag, if it's ready within the
 * budget. Never starts a capture: that would delay the tear-off itself.
 */
export async function takeTearOffSnapshot(blockId: string): Promise<string | undefined> {
    const h = held;
    held = null;
    if (!h || h.blockId !== blockId || Date.now() - h.at > MAX_AGE_MS) return undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const budget = new Promise<null>((resolve) => {
        timer = setTimeout(() => resolve(null), TAKE_BUDGET_MS);
    });
    const picture = await Promise.race([h.picture, budget]);
    clearTimeout(timer);
    if (picture && picture.length > MAX_SNAPSHOT_CHARS) {
        Logger.info("dnd:cross", "tear-off snapshot too large, dropped", { blockId, chars: picture.length });
        return undefined;
    }
    return picture ?? undefined;
}

/**
 * The pane a pointerdown may be about to drag out of its window: a
 * pragmatic draggable (a pane header or a Pane Tab pill) inside a pane.
 */
export function paneDragCandidate(target: EventTarget | null): string | null {
    if (!(target instanceof Element) || !target.closest('[draggable="true"]')) return null;
    return target.closest("[data-blockid]")?.getAttribute("data-blockid") ?? null;
}

/** Test hook. */
export function resetTearOffSnapshotForTests(): void {
    held = null;
}
