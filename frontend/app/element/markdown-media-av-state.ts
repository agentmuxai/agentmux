// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The inline video and audio player's state: what it has fetched (a poster range, then the whole file), the blob
 * URLs it owns and frees, and the lazy start both it and the local image share.
 */

import {
    describeMediaError,
    fetchMediaBlob,
    fetchMediaRange,
    INLINE_AV_MAX_BYTES,
    MediaTooLargeError,
} from "@/app/element/local-media";
import { createSignal, onCleanup, onMount } from "solid-js";

/** Start loading once within this distance of the viewport. */
const LAZY_MARGIN = "600px 0px";

/**
 * Calls `start` once `el` nears the viewport, or right away where there's no
 * IntersectionObserver. Returns a disposer.
 */
export function whenNearViewport(el: Element, start: () => void): () => void {
    if (typeof IntersectionObserver === "undefined") {
        start();
        return () => {};
    }
    const io = new IntersectionObserver(
        (entries) => {
            if (!entries.some((e) => e.isIntersecting)) return;
            io.disconnect();
            start();
        },
        { rootMargin: LAZY_MARGIN },
    );
    io.observe(el);
    return () => io.disconnect();
}

/** What a video reads before play, for a poster frame (§5). */
const POSTER_BYTES = 2 * 1024 * 1024;

type AvState =
    | { kind: "waiting" }
    | { kind: "ready"; total: number }
    | { kind: "loading"; total: number }
    | { kind: "playing"; url: string }
    | { kind: "too-large"; size: number }
    | { kind: "missing" }
    | { kind: "error"; message: string };

export type LocalAvState = ReturnType<typeof createLocalAv>;

export function createLocalAv(props: { path: string; src: string; kind: "video" | "audio" }) {
    const [state, setState] = createSignal<AvState>({ kind: "waiting" });
    const [poster, setPoster] = createSignal<string>();
    const [posterFailed, setPosterFailed] = createSignal(false);
    let box: HTMLElement | undefined;
    let disposed = false;
    const urls: string[] = [];
    const abort = new AbortController();
    let stopWatching = () => {};
    const own = (blob: Blob) => {
        const url = URL.createObjectURL(blob);
        urls.push(url);
        return url;
    };
    const fail = (e: unknown) => {
        if (disposed) return;
        setState(e instanceof MediaTooLargeError ? { kind: "too-large", size: e.size } : { kind: "missing" });
    };

    const probe = async () => {
        try {
            const end = props.kind === "video" ? POSTER_BYTES - 1 : 0;
            const { blob, total } = await fetchMediaRange(props.path, 0, end, { signal: abort.signal });
            if (disposed) return;
            if (total > INLINE_AV_MAX_BYTES) return setState({ kind: "too-large", size: total });
            if (props.kind === "video" && blob) setPoster(own(blob));
            setState({ kind: "ready", total });
        } catch (e) {
            fail(e);
        }
    };

    const play = async () => {
        const s = state();
        if (s.kind !== "ready") return;
        setState({ kind: "loading", total: s.total });
        try {
            const blob = await fetchMediaBlob(props.path, { maxBytes: INLINE_AV_MAX_BYTES, signal: abort.signal });
            if (disposed) return;
            setState({ kind: "playing", url: own(blob) });
        } catch (e) {
            fail(e);
        }
    };

    onMount(() => {
        if (box) stopWatching = whenNearViewport(box, () => void probe());
    });
    onCleanup(() => {
        disposed = true;
        stopWatching();
        abort.abort();
        for (const url of urls) URL.revokeObjectURL(url);
    });

    const onMediaError = (e: Event & { currentTarget: HTMLMediaElement }) => {
        const message = describeMediaError(e.currentTarget);
        // Free the played file (up to 200 MB) now, not when the row unmounts.
        const s = state();
        if (s.kind === "playing") {
            URL.revokeObjectURL(s.url);
            urls.splice(urls.indexOf(s.url), 1);
        }
        setState({ kind: "error", message });
    };
    const total = () => {
        const s = state();
        return s.kind === "ready" || s.kind === "loading" ? s.total : 0;
    };
    const playingUrl = () => {
        const s = state();
        return s.kind === "playing" ? s.url : undefined;
    };
    const errorState = () => {
        const s = state();
        return s.kind === "error" ? s.message : "";
    };
    const tooLargeSize = () => {
        const s = state();
        return s.kind === "too-large" ? s.size : 0;
    };

    return {
        state,
        poster,
        posterFailed,
        setPosterFailed,
        setBox: (el: HTMLElement) => {
            box = el;
        },
        play,
        onMediaError,
        total,
        playingUrl,
        errorState,
        tooLargeSize,
    };
}
