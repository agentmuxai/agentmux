// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    basenameOf,
    describeMediaError,
    extOf,
    fetchMediaBlob,
    fetchMediaRange,
    formatBytes,
    INLINE_AV_MAX_BYTES,
    INLINE_IMAGE_MAX_BYTES,
    inlineMediaKind,
    isNetworkPath,
    MediaTooLargeError,
    resolveMediaPath,
} from "@/app/element/local-media";
import { MarkdownContentBlockType } from "@/app/element/markdown-util";
import { createBlock } from "@/app/store/block-layout-actions";
import { openLink } from "@/app/store/global";
import { withHeightContinuity } from "@/app/view/agent/resize-contract";
import { fireAndForget } from "@/util/util";
import { createSignal, Match, onCleanup, onMount, Show, Switch, type JSX } from "solid-js";

/**
 * Turns on inline media for one `<Markdown>`: the agent's own messages only
 * (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §4.1). Tool results and file
 * previews never pass it, so text the agent didn't write can't make the pane
 * fetch anything.
 */
export type MarkdownMediaOpts = {
    /** Relative paths resolve against this: the agent's working directory. */
    baseDir: string;
};

interface MuxBlockProps {
    blockkey: string;
    blockmap: Map<string, MarkdownContentBlockType>;
}

const MuxBlock = (props: MuxBlockProps) => {
    const { blockkey, blockmap } = props;
    const block = blockmap.get(blockkey);
    if (block == null) {
        return null;
    }
    const sizeInKB = Math.round((block.content.length / 1024) * 10) / 10;
    const displayName = block.id.replace(/^"|"$/g, "");
    return (
        <div class="waveblock">
            <div class="wave-block-content">
                <div class="wave-block-icon">
                    <i class="fas fa-file-code"></i>
                </div>
                <div class="wave-block-info">
                    <span class="wave-block-filename">{displayName}</span>
                    <span class="wave-block-size">{sizeInKB} KB</span>
                </div>
            </div>
        </div>
    );
};

/** Same action as the `OpenMedia` tool: a new Media pane on the file. */
export function openInMediaPane(path: string): void {
    fireAndForget(() => createBlock({ meta: { view: "media", "media:path": path } }));
}

/**
 * Aspect ratios of images already shown, by path. A row the virtual list
 * unmounts and remounts reserves the right height straight away instead of
 * settling from the 16:9 placeholder again.
 */
const knownRatio = new Map<string, string>();

/**
 * Longest to wait for the size before showing the image anyway. A very large
 * image can take seconds to decode; past this it swaps in without a known
 * ratio and settles from normal layout instead.
 */
const RATIO_WAIT_MS = 1500;

async function naturalRatio(url: string): Promise<string | undefined> {
    if (typeof Image === "undefined") return undefined;
    const img = new Image();
    img.src = url;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timedOut = new Promise<false>((resolve) => (timer = setTimeout(() => resolve(false), RATIO_WAIT_MS)));
    try {
        const decoded = await Promise.race([img.decode().then(() => true), timedOut]);
        if (!decoded) return undefined;
    } catch {
        return undefined;
    } finally {
        clearTimeout(timer);
    }
    return img.naturalWidth && img.naturalHeight ? `${img.naturalWidth} / ${img.naturalHeight}` : undefined;
}

/** Start loading once within this distance of the viewport. */
const LAZY_MARGIN = "600px 0px";

/**
 * Calls `start` once `el` nears the viewport, or right away where there's no
 * IntersectionObserver. Returns a disposer.
 */
function whenNearViewport(el: Element, start: () => void): () => void {
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

type LocalImageState =
    | { kind: "waiting" }
    | { kind: "ready"; url: string }
    | { kind: "too-large"; size: number }
    | { kind: "missing" };

/**
 * A local image file (§4.2): a 16:9 placeholder until it nears the viewport,
 * then fetched through srv's authed route into a blob URL (revoked on
 * unmount). The natural aspect ratio is set before the image swaps in, and the
 * swap goes through the resize contract, so the row settles once.
 */
function LocalImage(props: { path: string; src: string; alt: string }): JSX.Element {
    const [state, setState] = createSignal<LocalImageState>({ kind: "waiting" });
    const [ratio, setRatio] = createSignal(knownRatio.get(props.path));
    let box: HTMLElement | undefined;
    let url: string | undefined;
    let disposed = false;
    const abort = new AbortController();
    let stopWatching = () => {};

    const load = async () => {
        try {
            const blob = await fetchMediaBlob(props.path, {
                maxBytes: INLINE_IMAGE_MAX_BYTES,
                type: extOf(props.path) === "svg" ? "image/svg+xml" : undefined,
                signal: abort.signal,
            });
            if (disposed) return;
            const objectUrl = URL.createObjectURL(blob);
            url = objectUrl;
            const r = await naturalRatio(objectUrl);
            if (disposed) return;
            const swap = () => {
                if (r) {
                    knownRatio.set(props.path, r);
                    setRatio(r);
                }
                setState({ kind: "ready", url: objectUrl });
            };
            if (box) withHeightContinuity(box, swap);
            else swap();
        } catch (e) {
            if (disposed) return;
            setState(e instanceof MediaTooLargeError ? { kind: "too-large", size: e.size } : { kind: "missing" });
        }
    };

    onMount(() => {
        if (box) stopWatching = whenNearViewport(box, () => void load());
    });
    onCleanup(() => {
        disposed = true;
        stopWatching();
        abort.abort();
        if (url) URL.revokeObjectURL(url);
    });

    const readyUrl = () => {
        const s = state();
        return s.kind === "ready" ? s.url : undefined;
    };
    const tooLargeSize = () => {
        const s = state();
        return s.kind === "too-large" ? s.size : 0;
    };

    return (
        <Switch
            fallback={
                <figure class="am-media" ref={box}>
                    <Show
                        when={readyUrl()}
                        fallback={<div class="am-media-placeholder" style={{ "aspect-ratio": ratio() ?? "16 / 9" }} />}
                    >
                        {(src) => (
                            <img
                                class="am-media-img"
                                src={src()}
                                alt={props.alt}
                                title="Open in Media pane"
                                style={ratio() ? { "aspect-ratio": ratio() } : undefined}
                                onClick={() => openInMediaPane(props.path)}
                                onError={() => {
                                    // Undecodable: free the blob now, not at unmount.
                                    if (url) URL.revokeObjectURL(url);
                                    url = undefined;
                                    setState({ kind: "missing" });
                                }}
                            />
                        )}
                    </Show>
                    <Show when={props.alt}>
                        <figcaption>{props.alt}</figcaption>
                    </Show>
                </figure>
            }
        >
            <Match when={state().kind === "missing"}>
                <span class="am-muted">[image not found: {props.src}]</span>
            </Match>
            <Match when={state().kind === "too-large"}>
                <MediaCard path={props.path} size={tooLargeSize()} icon="fa-image" />
            </Match>
        </Switch>
    );
}

/** Over the inline cap: the file's name and size, opening a Media pane. */
function MediaCard(props: { path: string; size: number; icon: string }): JSX.Element {
    return (
        <button type="button" class="am-media-card" onClick={() => openInMediaPane(props.path)}>
            <i class={`fa ${props.icon}`} aria-hidden="true" />
            <span class="am-media-card-name">{basenameOf(props.path)}</span>
            <span class="am-muted">{formatBytes(props.size)}</span>
            <span class="am-media-card-action">Open in Media pane</span>
        </button>
    );
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

/**
 * A local video or audio file (§5). Nothing autoplays on its own. Before play
 * it reads only a small range: a video's first 2 MB for a poster frame (a file
 * whose index is at the end shows a plain placeholder instead), an audio
 * file's first byte for its size. Pressing play fetches the whole file, under
 * the 200 MB cap, and plays it (video muted, with controls). A video keeps its
 * reserved 16:9 stage throughout, so its row never changes height.
 */
function LocalAV(props: { path: string; src: string; kind: "video" | "audio" }): JSX.Element {
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
    const label = () => (
        <span class="am-media-av-label">
            <span class="am-media-card-name">{basenameOf(props.path)}</span>
            <Show when={total()}>
                <span class="am-muted">{formatBytes(total())}</span>
            </Show>
        </span>
    );
    // Appears once the size is known (and under the cap): nothing to press before.
    const playButton = () => (
        <Show when={state().kind === "ready" || state().kind === "loading"}>
            <button
                type="button"
                class="am-media-play"
                disabled={state().kind !== "ready"}
                onClick={() => void play()}
                aria-label={`Play ${basenameOf(props.path)}`}
            >
                <i class={state().kind === "loading" ? "fa fa-spinner fa-spin" : "fa fa-play"} aria-hidden="true" />
            </button>
        </Show>
    );
    const errorState = () => {
        const s = state();
        return s.kind === "error" ? s.message : "";
    };
    const tooLargeSize = () => {
        const s = state();
        return s.kind === "too-large" ? s.size : 0;
    };

    return (
        <Switch
            fallback={
                <figure class={`am-media am-media-${props.kind}`} ref={box}>
                    <Show
                        when={props.kind === "video"}
                        fallback={
                            <Show when={playingUrl()} fallback={<div class="am-media-audio-card">{playButton()}{label()}</div>}>
                                {(url) => <audio class="am-media-player" src={url()} controls autoplay onError={onMediaError} />}
                            </Show>
                        }
                    >
                        <div class="am-media-stage">
                            <Show
                                when={playingUrl()}
                                fallback={
                                    <>
                                        <Show when={poster() && !posterFailed()}>
                                            <video
                                                class="am-media-poster"
                                                // `#t=0.1`: seek past a black first frame; `auto`:
                                                // the bytes are already in memory, so decode one.
                                                src={`${poster()}#t=0.1`}
                                                preload="auto"
                                                muted
                                                playsinline
                                                onError={() => setPosterFailed(true)}
                                            />
                                        </Show>
                                        <div class="am-media-stage-overlay">
                                            {playButton()}
                                            {label()}
                                        </div>
                                    </>
                                }
                            >
                                {(url) => (
                                    <video
                                        // Starts muted (§5); the property, not just
                                        // the attribute, so it holds whatever loads.
                                        ref={(el) => (el.muted = true)}
                                        class="am-media-player"
                                        src={url()}
                                        controls
                                        autoplay
                                        muted
                                        playsinline
                                        onError={onMediaError}
                                    />
                                )}
                            </Show>
                        </div>
                    </Show>
                </figure>
            }
        >
            <Match when={state().kind === "missing"}>
                <span class="am-muted">
                    [{props.kind} not found: {props.src}]
                </span>
            </Match>
            <Match when={state().kind === "error"}>
                <span class="am-muted">
                    [{props.kind} can't play: {errorState()}]
                </span>
            </Match>
            <Match when={state().kind === "too-large"}>
                <MediaCard path={props.path} size={tooLargeSize()} icon={props.kind === "video" ? "fa-film" : "fa-music"} />
            </Match>
        </Switch>
    );
}

/**
 * An `https://` or `http://` image (§4.3): a chip until clicked. Loading it
 * automatically would let a prompt-injected agent exfiltrate data in the URL
 * the moment the message renders.
 */
function RemoteImage(props: { src: string; alt: string }): JSX.Element {
    const [load, setLoad] = createSignal(false);
    let host = props.src;
    try {
        host = new URL(props.src).host;
    } catch {
        // Keep the raw text; the chip still says what it would load.
    }
    const insecure = /^http:/i.test(props.src);
    return (
        <Show
            when={load()}
            fallback={
                <button type="button" class="am-media-chip" onClick={() => setLoad(true)} title={props.src}>
                    🌐 image from {host} — load{insecure ? " (http, not encrypted)" : ""}
                </button>
            }
        >
            <figure class="am-media">
                <img class="am-media-img" src={props.src} alt={props.alt} referrerPolicy="no-referrer" />
                <Show when={props.alt}>
                    <figcaption>{props.alt}</figcaption>
                </Show>
            </figure>
        </Show>
    );
}

/**
 * Raster data URIs only. An SVG can reference remote resources, so an inline
 * SVG data URI never renders (ReAgent P0 on #4064); a local `.svg` file goes
 * through the blob path like any other file.
 */
const RASTER_DATA_URI = /^data:image\/(?:png|jpe?g|gif|webp)[;,]/i;

/**
 * Every markdown `<img>`. Nothing renders unless `media` is on — the agent's
 * own messages. There, a raster `data:` URI renders directly (no request).
 */
const MarkdownImg = (p: { props: JSX.ImgHTMLAttributes<HTMLImageElement>; media?: MarkdownMediaOpts }) => {
    const src = ((p.props as any)?.src as string | undefined) ?? "";
    const alt = ((p.props as any)?.alt as string | undefined) ?? "";
    if (!p.media) return <span>[img:{src.startsWith("data:") ? "data" : src}]</span>;
    if (RASTER_DATA_URI.test(src)) return <img src={src} alt={alt} />;
    if (/^data:/i.test(src)) return <span class="am-muted">[image: inline data — unsupported type]</span>;
    if (/^https?:\/\//i.test(src)) {
        // Remote video/audio isn't supported inline (§5): a link, never loaded.
        const remoteKind = inlineMediaKind(src.split(/[?#]/)[0]);
        if (remoteKind === "video" || remoteKind === "audio") {
            return (
                <a
                    href={src}
                    onClick={(e) => {
                        e.preventDefault();
                        openLink(src);
                    }}
                >
                    {alt || src}
                </a>
            );
        }
        return <RemoteImage src={src} alt={alt} />;
    }
    const path = resolveMediaPath(src, p.media.baseDir);
    if (path == null) return <span class="am-muted">[image not found: {src}]</span>;
    // A drive path arrives as the file:/// URL rehype-local-image-src made of
    // it; name it the way the agent wrote it.
    const shown = /^file:/i.test(src) ? path : src;
    if (isNetworkPath(path)) return <span class="am-muted">[image: {shown} — network paths aren't loaded]</span>;
    const kind = inlineMediaKind(path);
    if (kind === "image") return <LocalImage path={path} src={shown} alt={alt} />;
    if (kind === "video" || kind === "audio") return <LocalAV path={path} src={shown} kind={kind} />;
    return <span class="am-muted">[image: {shown} — unsupported type]</span>;
};

export { MarkdownImg, MuxBlock };
