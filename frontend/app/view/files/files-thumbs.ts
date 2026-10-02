// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Thumbnails for the Files pane's grid view (spec §5.2): an image file read
 * once, scaled down to a small bitmap, and kept as a small object URL, so a
 * folder of 20 MB photos costs a few kilobytes each once shown. At most a
 * few are made at a time, and a bounded number are kept.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.2 (grid).
 */

import { fetchMediaBlob, IMAGE_EXTENSIONS, INLINE_IMAGE_MAX_BYTES } from "@/app/element/local-media";
import { extensionOf } from "./files-sort";

/** The longer side of a thumbnail, in pixels (2× a 96 px tile). */
export const THUMB_PX = 192;
/** Thumbnails kept; past this the least recently used is released. */
const MAX_CACHED = 300;
/** Images decoded at once. */
const MAX_ACTIVE = 4;

const cache = new Map<string, string>();
const inflight = new Map<string, Promise<string | null>>();
let active = 0;
const waiting: (() => void)[] = [];

/** Whether a file gets a thumbnail rather than an icon. */
export function hasThumbnail(name: string, size: number | undefined): boolean {
    return IMAGE_EXTENSIONS.includes(extensionOf(name)) && (size ?? 0) > 0 && (size ?? 0) <= INLINE_IMAGE_MAX_BYTES;
}

const keyOf = (path: string, mtime?: number) => `${path}\u0000${mtime ?? ""}`;

/** A cached thumbnail, without making one. */
export function cachedThumbnail(path: string, mtime?: number): string | undefined {
    const key = keyOf(path, mtime);
    const url = cache.get(key);
    if (url) {
        // Most recently used goes last.
        cache.delete(key);
        cache.set(key, url);
    }
    return url;
}

async function slot<T>(work: () => Promise<T>): Promise<T> {
    if (active >= MAX_ACTIVE) await new Promise<void>((r) => waiting.push(r));
    active++;
    try {
        return await work();
    } finally {
        active--;
        waiting.shift()?.();
    }
}

/** Scale `blob` so its longer side is THUMB_PX; the blob itself when it is
 *  already small or can't be decoded as a bitmap (SVG in some engines). */
async function shrink(blob: Blob): Promise<Blob> {
    if (typeof createImageBitmap !== "function") return blob;
    let probe: ImageBitmap;
    try {
        probe = await createImageBitmap(blob);
    } catch {
        return blob;
    }
    const { width, height } = probe;
    probe.close();
    if (width <= THUMB_PX && height <= THUMB_PX) return blob;
    const scale = THUMB_PX / Math.max(width, height);
    const w = Math.max(1, Math.round(width * scale));
    const h = Math.max(1, Math.round(height * scale));
    const bitmap = await createImageBitmap(blob, { resizeWidth: w, resizeHeight: h, resizeQuality: "medium" });
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
    bitmap.close();
    return await new Promise<Blob>((resolve) => canvas.toBlob((b) => resolve(b ?? blob), "image/png"));
}

/** The thumbnail for an image file, made if needed; null when it can't be. */
export function thumbnail(path: string, mtime?: number): Promise<string | null> {
    const key = keyOf(path, mtime);
    const hit = cachedThumbnail(path, mtime);
    if (hit) return Promise.resolve(hit);
    const running = inflight.get(key);
    if (running) return running;
    const job = slot(async () => {
        try {
            const ext = extensionOf(path);
            const blob = await fetchMediaBlob(path, {
                maxBytes: INLINE_IMAGE_MAX_BYTES,
                type: ext === "svg" ? "image/svg+xml" : undefined,
            });
            const url = URL.createObjectURL(await shrink(blob));
            cache.set(key, url);
            while (cache.size > MAX_CACHED) {
                const [oldKey, oldUrl] = cache.entries().next().value!;
                cache.delete(oldKey);
                URL.revokeObjectURL(oldUrl);
            }
            return url;
        } catch {
            return null;
        } finally {
            inflight.delete(key);
        }
    });
    inflight.set(key, job);
    return job;
}

/** Test hook. */
export function resetThumbnailsForTests(): void {
    for (const url of cache.values()) URL.revokeObjectURL(url);
    cache.clear();
    inflight.clear();
    active = 0;
    waiting.length = 0;
}
