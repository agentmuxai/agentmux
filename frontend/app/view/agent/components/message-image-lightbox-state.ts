// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The message preview's state: which image or video is shown, the original file of it and, for images, of the
 * one on each side (loaded through the same authed route and cap as the inline image), and the keys
 * (SPEC_AGENT_PANE_IMAGE_ARROWS_2026_11_06.md §3).
 */

import {
    extOf,
    fetchMediaBlob,
    INLINE_AV_MAX_BYTES,
    INLINE_IMAGE_MAX_BYTES,
    MediaTooLargeError,
} from "@/app/element/local-media";
import type { MediaGalleryItem, VideoHandoff } from "@/app/element/markdown-media";
import { createEffect, createSignal, on, onCleanup } from "solid-js";

/** Where an enlarged video ended: the inline player for the same file carries on from here (paused). */
export type VideoHandback = { path: string; time: number; muted: boolean };

export type MessageImageLightboxProps = {
    items: MediaGalleryItem[];
    /** The image or video the person clicked. */
    index: number;
    /** Set when a playing inline video was enlarged: the enlarged one starts from its time and sound. */
    start?: VideoHandoff;
    /** `back` is set when a video was on show: where it was. */
    onClose: (back?: VideoHandback) => void;
    /** Close, then open this file in the Editor (zoom and pan live there). */
    onOpenEditor: (path: string) => void;
    /** The agent pane the message is in: srv reads a container agent's paths from its own folders only. */
    blockId?: string;
};

const isVideo = (item: MediaGalleryItem | undefined): boolean => item?.kind === "video";

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
function createImageStore(abort: AbortController, videoPaths: () => Set<string>, blockId?: () => string | undefined) {
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
                maxBytes: videoPaths().has(path) ? INLINE_AV_MAX_BYTES : INLINE_IMAGE_MAX_BYTES,
                type: extOf(path) === "svg" ? "image/svg+xml" : undefined,
                signal: abort.signal,
                blockId: blockId?.(),
            });
            // Let go of a result nobody is waiting for: the entry was dropped while it loaded.
            if (abort.signal.aborted || entries()[path]?.kind !== "loading") return;
            put(path, { kind: "ready", url: URL.createObjectURL(blob), bytes: blob.size });
        } catch (e) {
            if (abort.signal.aborted || entries()[path]?.kind !== "loading") return;
            put(path, { kind: e instanceof MediaTooLargeError ? "too-large" : "missing" });
        }
    };
    /**
     * Lets go of every file that is not within one step of position `i` of `paths` (a repeated file stays while
     * any of its places is). A video is only ever kept while it is the one shown: it can be 200 MB.
     */
    const dropFarFrom = (i: number, paths: string[]) => {
        const videos = videoPaths();
        const near = new Set(
            paths.filter((p, pos) => (videos.has(p) ? pos === i : ringDistance(pos, i, paths.length) <= 1)),
        );
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
function previewKeys(k: {
    close: () => void;
    go: (to: number) => void;
    at: () => number;
    last: () => number;
    editor: () => void;
    actualSize: () => void;
    playPause: () => void;
    seek: (seconds: number) => void;
}) {
    const keys: Record<string, () => void> = {
        Escape: k.close,
        ArrowLeft: () => k.go(k.at() - 1),
        ArrowRight: () => k.go(k.at() + 1),
        Home: () => k.go(0),
        End: () => k.go(k.last()),
        e: k.editor,
        E: k.editor,
        "1": k.actualSize,
        " ": k.playPause,
        j: () => k.seek(-10),
        J: () => k.seek(-10),
        l: () => k.seek(10),
        L: () => k.seek(10),
    };
    return (e: KeyboardEvent) => {
        // A button or the player itself already acts on Space (and Enter) when it has the focus.
        if (e.key === " " && (e.target instanceof HTMLButtonElement || e.target instanceof HTMLVideoElement)) return;
        const act = e.ctrlKey || e.metaKey || e.altKey ? undefined : keys[e.key];
        if (!act) return;
        e.preventDefault();
        // So Escape doesn't also close whatever is under the preview.
        if (e.key === "Escape") e.stopPropagation();
        act();
    };
}

/**
 * The enlarged player: the element on show, where an inline player left off (used once, for the file it was taken
 * from), and the keys' play, pause and skip.
 */
function createVideoPlayer(item: () => MediaGalleryItem | undefined, start: VideoHandoff | undefined) {
    let el: HTMLVideoElement | undefined;
    let handoff = start;
    return {
        /** A video element just mounted for the shown item: it carries on from the inline player's time and sound. */
        setVideo: (video: HTMLVideoElement) => {
            el = video;
            video.muted = true;
            const from = handoff;
            if (!from || from.path !== item()?.path) return;
            handoff = undefined;
            video.muted = from.muted;
            video.addEventListener(
                "loadedmetadata",
                () => {
                    video.currentTime = from.time;
                    if (!from.paused) void video.play().catch(() => {});
                },
                { once: true },
            );
        },
        /** The shown video's place and sound, for the inline player to carry on from. */
        read: (): VideoHandback | undefined => {
            const here = item();
            if (!el?.isConnected || !isVideo(here)) return undefined;
            return { path: here.path, time: el.currentTime, muted: el.muted };
        },
        playPause: () => {
            if (!el?.isConnected) return;
            if (el.paused) void el.play().catch(() => {});
            else el.pause();
        },
        seek: (seconds: number) => {
            if (!el?.isConnected) return;
            el.currentTime = Math.max(0, Math.min(el.duration || Infinity, el.currentTime + seconds));
        },
    };
}

export function createMessageLightbox(props: MessageImageLightboxProps) {
    const [index, setIndex] = createSignal(props.index);
    const [sizes, setSizes] = createSignal<Record<string, string>>({});
    const [actual, setActual] = createSignal(false);
    const abort = new AbortController();
    const store = createImageStore(abort, () => new Set(props.items.filter(isVideo).map((it) => it.path)), () => props.blockId);
    let dialogRef: HTMLDivElement | undefined;

    const n = () => props.items.length;
    const item = () => props.items[Math.min(index(), n() - 1)];
    const player = createVideoPlayer(item, props.start);
    const at = (i: number): MediaGalleryItem | undefined => props.items[((i % n()) + n()) % n()];
    const entry = (): ImageEntry | undefined => (item() ? store.entries()[item()!.path] : undefined);

    // The shown image first, then the one on each side once it has settled, so a step is instant. An
    // image two steps away is let go.
    createEffect(
        on(index, (i) => {
            const here = item();
            if (!here) return;
            setActual(false);
            void store.load(here.path).then(() => {
                if (index() !== i) return;
                for (const d of [1, -1]) {
                    const near = at(i + d);
                    // A neighbouring video is read when it is stepped to, not ahead of time.
                    if (near && !isVideo(near)) void store.load(near.path);
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
    const close = () => props.onClose(player.read());
    const openEditor = () => {
        const here = item();
        if (here) props.onOpenEditor(here.path);
    };
    return {
        index,
        item,
        entry,
        actual,
        toggleActual: () => !isVideo(item()) && setActual((a) => !a),
        isVideo: () => isVideo(item()),
        setVideo: player.setVideo,
        close,
        label: () => {
            const here = item();
            return `${isVideo(here) ? "Video" : "Image"} ${Math.min(index(), n() - 1) + 1} of ${n()}: ${here?.alt || here?.name}`;
        },
        size: () => (item() ? sizes()[item()!.path] : undefined),
        setSize: (path: string, w: number, h: number) => setSizes((all) => ({ ...all, [path]: `${w}×${h}` })),
        count: n,
        go,
        openEditor,
        onKeyDown: previewKeys({
            close,
            go,
            at: index,
            last: () => n() - 1,
            editor: openEditor,
            actualSize: () => !isVideo(item()) && setActual((on) => !on),
            playPause: player.playPause,
            seek: player.seek,
        }),
        setDialog: (el: HTMLDivElement) => {
            dialogRef = el;
        },
        dialog: () => dialogRef,
    };
}
