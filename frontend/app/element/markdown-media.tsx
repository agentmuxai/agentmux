// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    basenameOf,
    extOf,
    fetchMediaBlob,
    formatBytes,
    INLINE_IMAGE_MAX_BYTES,
    inlineMediaKind,
    MediaTooLargeError,
    resolveMediaPath,
} from "@/app/element/local-media";
import { MarkdownContentBlockType } from "@/app/element/markdown-util";
import { createBlock } from "@/app/store/block-layout-actions";
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
function openInMediaPane(path: string): void {
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
                                onError={() => setState({ kind: "missing" })}
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
                <button type="button" class="am-media-card" onClick={() => openInMediaPane(props.path)}>
                    <i class="fa fa-image" aria-hidden="true" />
                    <span class="am-media-card-name">{basenameOf(props.path)}</span>
                    <span class="am-muted">{formatBytes(tooLargeSize())}</span>
                    <span class="am-media-card-action">Open in Media pane</span>
                </button>
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
 * Every markdown `<img>`. A `data:image/` URI renders directly, as it always
 * has (it makes no request). Anything else loads only where `media` is on.
 */
const MarkdownImg = (p: { props: JSX.ImgHTMLAttributes<HTMLImageElement>; media?: MarkdownMediaOpts }) => {
    const src = ((p.props as any)?.src as string | undefined) ?? "";
    const alt = ((p.props as any)?.alt as string | undefined) ?? "";
    if (src.startsWith("data:image/")) return <img src={src} alt={alt} />;
    if (!p.media) return <span>[img:{src}]</span>;
    if (/^https?:\/\//i.test(src)) return <RemoteImage src={src} alt={alt} />;
    const path = resolveMediaPath(src, p.media.baseDir);
    if (path == null) return <span class="am-muted">[image not found: {src}]</span>;
    // A drive path arrives as the file:/// URL rehype-local-image-src made of
    // it; name it the way the agent wrote it.
    const shown = /^file:/i.test(src) ? path : src;
    if (inlineMediaKind(path) === "image") return <LocalImage path={path} src={shown} alt={alt} />;
    return <span class="am-muted">[image: {shown} — unsupported type]</span>;
};

export { MarkdownImg, MuxBlock };
