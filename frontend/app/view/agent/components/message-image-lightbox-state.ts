// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The message image preview's state: which image is shown, the original file of it and of the
 * one on each side (loaded through the same authed route and cap as the inline image), and the keys
 * (SPEC_AGENT_PANE_IMAGE_ARROWS_2026_11_06.md §3).
 */

import { extOf, fetchMediaBlob, INLINE_IMAGE_MAX_BYTES, MediaTooLargeError } from "@/app/element/local-media";
import type { MediaGalleryItem } from "@/app/element/markdown-media";
import { createEffect, createSignal, on, onCleanup } from "solid-js";

export type MessageImageLightboxProps = {
    items: MediaGalleryItem[];
    /** The image the person clicked. */
    index: number;
    onClose: () => void;
    /** Close, then open this file in the Editor (zoom and pan live there). */
    onOpenEditor: (path: string) => void;
};

/** One file's load: a blob URL and what is known of it, or why there is none. */
export type ImageEntry =
    | { kind: "loading" }
    | { kind: "ready"; url: string; bytes: number }
    | { kind: "too-large" }
    | { kind: "missing" };

export type MessageLightboxState = ReturnType<typeof createMessageLightbox>;

/** Steps between two positions of a ring of `n`. */
export function ringDistance(a: number, b: number, n: number): number {
    const d = Math.abs(a - b);
    return Math.min(d, n - d);
}

/** The files loaded so far, by path: each one's blob URL or why there is none. */
function createImageStore(abort: AbortController) {
    const [entries, setEntries] = createSignal<Record<string, ImageEntry>>({});
    const revoke = (e: ImageEntry | undefined) => {
        if (e?.kind === "ready") URL.revokeObjectURL(e.url);
    };
    const put = (path: string, e: ImageEntry | undefined) =>
        setEntries((all) => {
            const next = { ...all };
            if (e) next[path] = e;
            else delete next[path];
            return next;
        });
    const load = async (path: string): Promise<void> => {
        if (entries()[path]) return;
        put(path, { kind: "loading" });
        try {
            const blob = await fetchMediaBlob(path, {
                maxBytes: INLINE_IMAGE_MAX_BYTES,
                type: extOf(path) === "svg" ? "image/svg+xml" : undefined,
                signal: abort.signal,
            });
            // Let go of a result nobody is waiting for: the entry was dropped while it loaded.
            if (abort.signal.aborted || entries()[path]?.kind !== "loading") return;
            put(path, { kind: "ready", url: URL.createObjectURL(blob), bytes: blob.size });
        } catch (e) {
            if (abort.signal.aborted || entries()[path]?.kind !== "loading") return;
            put(path, { kind: e instanceof MediaTooLargeError ? "too-large" : "missing" });
        }
    };
    /** Lets go of every file that is not within one step of position `i` of `paths` (a repeated file stays while any of its places is). */
    const dropFarFrom = (i: number, paths: string[]) => {
        const near = new Set(paths.filter((_, pos) => ringDistance(pos, i, paths.length) <= 1));
        for (const [path, e] of Object.entries(entries())) {
            if (paths.includes(path) && !near.has(path)) {
                revoke(e);
                put(path, undefined);
            }
        }
    };
    const releaseAll = () => Object.values(entries()).forEach(revoke);
    return { entries, load, dropFarFrom, releaseAll };
}

/** The keys the preview answers to; a modifier held means a shortcut, not ours. */
function previewKeys(k: { close: () => void; go: (to: number) => void; at: () => number; last: () => number; editor: () => void }) {
    const keys: Record<string, () => void> = {
        Escape: k.close,
        ArrowLeft: () => k.go(k.at() - 1),
        ArrowRight: () => k.go(k.at() + 1),
        Home: () => k.go(0),
        End: () => k.go(k.last()),
        e: k.editor,
        E: k.editor,
    };
    return (e: KeyboardEvent) => {
        const act = e.ctrlKey || e.metaKey || e.altKey ? undefined : keys[e.key];
        if (!act) return;
        e.preventDefault();
        // So Escape doesn't also close whatever is under the preview.
        if (e.key === "Escape") e.stopPropagation();
        act();
    };
}

export function createMessageLightbox(props: MessageImageLightboxProps) {
    const [index, setIndex] = createSignal(props.index);
    const [sizes, setSizes] = createSignal<Record<string, string>>({});
    const abort = new AbortController();
    const store = createImageStore(abort);
    let dialogRef: HTMLDivElement | undefined;

    const n = () => props.items.length;
    const at = (i: number): MediaGalleryItem | undefined => props.items[((i % n()) + n()) % n()];
    const item = () => props.items[Math.min(index(), n() - 1)];
    const entry = (): ImageEntry | undefined => (item() ? store.entries()[item()!.path] : undefined);

    // The shown image first, then the one on each side once it has settled, so a step is instant. An
    // image two steps away is let go.
    createEffect(
        on(index, (i) => {
            const here = item();
            if (!here) return;
            void store.load(here.path).then(() => {
                if (index() !== i) return;
                for (const d of [1, -1]) {
                    const near = at(i + d);
                    if (near) void store.load(near.path);
                }
            });
            store.dropFarFrom(i, props.items.map((it) => it.path));
        }),
    );
    onCleanup(() => {
        abort.abort();
        store.releaseAll();
    });

    const go = (to: number) => {
        if (n() > 0) setIndex(((to % n()) + n()) % n());
    };
    const openEditor = () => {
        const here = item();
        if (here) props.onOpenEditor(here.path);
    };

    return {
        index,
        item,
        entry,
        label: () => `Image ${Math.min(index(), n() - 1) + 1} of ${n()}: ${item()?.alt || item()?.name}`,
        size: () => (item() ? sizes()[item()!.path] : undefined),
        setSize: (path: string, w: number, h: number) => setSizes((all) => ({ ...all, [path]: `${w}×${h}` })),
        count: n,
        go,
        openEditor,
        onKeyDown: previewKeys({ close: props.onClose, go, at: index, last: () => n() - 1, editor: openEditor }),
        setDialog: (el: HTMLDivElement) => {
            dialogRef = el;
        },
        dialog: () => dialogRef,
    };
}
