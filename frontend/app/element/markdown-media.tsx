// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    basenameOf,
    NATIVE_PLAYER_CONTROLS_LIST,
    extOf,
    fetchMediaBlob,
    INLINE_IMAGE_MAX_BYTES,
    inlineMediaKind,
    isNetworkPath,
    MediaTooLargeError,
    refusalReason,
    resolveMediaPath,
    type MediaOwner,
} from "@/app/element/local-media";
import { showMediaCopyMenu, sourceFromUrl } from "@/app/element/media-copy";
import { openMessageFile } from "@/app/element/open-message-file";
import { t } from "@/app/i18n/i18n";
import { formatBytes } from "@/util/format-bytes";
import { MarkdownContentBlockType } from "@/app/element/markdown-util";
import { createBlock } from "@/app/store/block-layout-actions";
import { openLink } from "@/app/store/global";
import { withHeightContinuity } from "@/app/motion/resize-contract";
import { openInEditorPaneOnScreen } from "@/app/util/editor-open";
import { Button, IconButton } from "@/element/ui";
import { fireAndForget } from "@/util/util";
import { createSignal, Match, onCleanup, onMount, Show, Switch, type JSX } from "solid-js";
import { tip } from "@/app/element/tip";
import { createLocalAv, whenNearViewport, type LocalAvState } from "./markdown-media-av-state";

/**
 * Turns on inline media for one `<Markdown>`: the agent's own messages only
 * (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §4.1). Tool results and file
 * previews never pass it, so text the agent didn't write can't make the pane
 * fetch anything.
 */
export type MarkdownMediaOpts = {
    /** Relative paths resolve against this: the agent's working directory. */
    baseDir: string;
    /** The agent pane showing the message: srv reads a container agent's paths from its own folders (see `MediaOwner`). */
    blockId?: string;
    /** A click on a file of the message is opened by srv from the agent's own folders, not as a path of this computer. */
    confined?: boolean;
    /**
     * Set by the agent pane: a click on a local image of a message, or on a video's enlarge button, opens a
     * preview over the whole tab that steps through the message's images and videos. Without it a click on an
     * image opens the Editor.
     */
    openGallery?: (gallery: MediaGallery) => void;
};

/** One image or video of a message, for the preview that steps through them. */
export type MediaGalleryItem = { path: string; name: string; alt: string; kind?: "image" | "video" };
/** Where the inline player was when it was enlarged, so the enlarged one carries on from there. */
export type VideoHandoff = { path: string; time: number; muted: boolean; paused: boolean };
export type MediaGallery = { items: MediaGalleryItem[]; index: number; start?: VideoHandoff };

/** The block's text is one message: its images and videos, in reading order, and the position of `figure` among them. */
function messageGallery(figure: HTMLElement): MediaGallery {
    const figures = [...(figure.closest(".agent-markdown-block")?.querySelectorAll<HTMLElement>("figure[data-am-media-path]") ?? [])];
    return {
        items: figures.map((f) => {
            const path = f.dataset.amMediaPath ?? "";
            const kind: "image" | "video" = f.dataset.amMediaKind === "video" ? "video" : "image";
            return { path, name: basenameOf(path), alt: f.dataset.amMediaAlt ?? "", kind };
        }),
        index: figures.indexOf(figure),
    };
}

/** The inline players that are playing a file, by path: where the enlarged player starts, and where it hands back to. */
const inlineVideos = new Map<string, HTMLVideoElement>();

/** The enlarged player closed: the inline one for the same file is left where it ended, paused, with its sound as set. */
export function handBackVideo(h: { path: string; time: number; muted: boolean }): void {
    const el = inlineVideos.get(h.path);
    if (!el?.isConnected) return;
    el.pause();
    if (Number.isFinite(h.time)) el.currentTime = h.time;
    el.muted = h.muted;
}

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

/** Open an image or video in the Editor: a tab of the Editor pane on screen,
 *  else a new Editor pane, following newer files in its folder as the Media
 *  pane did (SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.12.1). */
export function openMediaInEditor(path: string, owner?: MediaOwner): void {
    // A file of a container agent's message is opened by srv, from the agent's own folders and as a read-only copy:
    // never as the path the message wrote, which this computer would read as the person.
    if (owner?.confined) {
        openMessageFile(path, owner.blockId);
        return;
    }
    fireAndForget(async () => {
        if (await openInEditorPaneOnScreen(path, { followFolder: true })) return;
        await createBlock({ meta: { view: "editor", file: path, "editor:follow_folder": true } });
    });
}

/** srv's reason for refusing a file, after the placeholder's text, when it gave one. */
function reasonOf(state: { kind: string; reason?: string }): string {
    return state.kind === "missing" && state.reason ? " " + state.reason : "";
}

type LocalImageProps = { path: string; src: string; alt: string; openGallery?: MarkdownMediaOpts["openGallery"]; owner?: MediaOwner };

/** A click on an image of an agent's message opens the preview over the tab; with no pane to show it, the Editor. */
function openImage(props: LocalImageProps, figure: HTMLElement | undefined): void {
    const gallery = props.openGallery && figure ? messageGallery(figure) : undefined;
    if (gallery && gallery.index >= 0) props.openGallery!(gallery);
    else openMediaInEditor(props.path, props.owner);
}

/** An image of an agent's message is a button for the keyboard too, named with its place among the message's images once focused. */
function galleryKeys(open: () => void, name: string) {
    return {
        tabIndex: 0,
        role: "button" as const,
        "aria-label": `Open image: ${name}`,
        onKeyDown: (e: KeyboardEvent) => {
            if (e.key !== "Enter" && e.key !== " ") return;
            e.preventDefault();
            open();
        },
        onFocus: (e: FocusEvent) => {
            const img = e.currentTarget as HTMLElement;
            const figure = img.closest<HTMLElement>("figure[data-am-media-path]");
            const { items, index } = figure ? messageGallery(figure) : { items: [], index: -1 };
            if (items.length > 1) img.setAttribute("aria-label", `Open image: ${name}, ${index + 1} of ${items.length}`);
        },
    };
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

type LocalImageState =
    | { kind: "waiting" }
    | { kind: "ready"; url: string }
    | { kind: "too-large"; size: number }
    | { kind: "missing"; reason?: string };

/**
 * A local image file (§4.2): a 16:9 placeholder until it nears the viewport,
 * then fetched through srv's authed route into a blob URL (revoked on
 * unmount). The natural aspect ratio is set before the image swaps in, and the
 * swap goes through the resize contract, so the row settles once.
 */
function LocalImage(props: LocalImageProps): JSX.Element {
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
                blockId: props.owner?.blockId,
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
            setState(e instanceof MediaTooLargeError ? { kind: "too-large", size: e.size } : { kind: "missing", reason: refusalReason(e) });
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

    const open = () => openImage(props, box);

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
                <figure class="am-media" ref={box} data-am-media-path={props.path} data-am-media-alt={props.alt}>
                    <Show
                        when={readyUrl()}
                        fallback={<div class="am-media-placeholder" style={{ "aspect-ratio": ratio() ?? "16 / 9" }} />}
                    >
                        {(src) => (
                            <img
                                class="am-media-img"
                                src={src()}
                                alt={props.alt}
                                {...tip(props.openGallery ? "Open image" : "Open in the editor")}
                                style={ratio() ? { "aspect-ratio": ratio() } : undefined}
                                {...(props.openGallery ? galleryKeys(open, props.alt || basenameOf(props.path)) : {})}
                                onClick={open}
                                onContextMenu={(e) => showMediaCopyMenu(e, sourceFromUrl(src(), { path: props.path }))}
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
                <span class="am-muted">[image not found: {props.src}]{reasonOf(state())}</span>
            </Match>
            <Match when={state().kind === "too-large"}>
                <MediaCard path={props.path} owner={props.owner} size={tooLargeSize()} icon="fa-image" />
            </Match>
        </Switch>
    );
}

/** Over the inline cap: the file's name and size, opening it in the Editor. */
function MediaCard(props: { path: string; size: number; icon: string; owner?: MediaOwner }): JSX.Element {
    return (
        <Button class="am-media-card" onClick={() => openMediaInEditor(props.path, props.owner)}>
            <i class={`fa ${props.icon}`} aria-hidden="true" />
            <span class="am-media-card-name">{basenameOf(props.path)}</span>
            <span class="am-muted">{formatBytes(props.size)}</span>
            <span class="am-media-card-action">Open in the editor</span>
        </Button>
    );
}

function AvLabel(props: { s: LocalAvState; path: string }): JSX.Element {
    return (
        <span class="am-media-av-label">
            <span class="am-media-card-name">{basenameOf(props.path)}</span>
            <Show when={props.s.total()}>
                <span class="am-muted">{formatBytes(props.s.total())}</span>
            </Show>
        </span>
    );
}

// Appears once the size is known (and under the cap): nothing to press before.
function PlayButton(props: { s: LocalAvState; path: string }): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.state().kind === "ready" || s.state().kind === "loading"}>
            <button
                type="button"
                class="am-media-play"
                disabled={s.state().kind !== "ready"}
                onClick={() => void s.play()}
                aria-label={`Play ${basenameOf(props.path)}`}
            >
                <i class={s.state().kind === "loading" ? "fa fa-spinner fa-spin" : "fa fa-play"} aria-hidden="true" />
            </button>
        </Show>
    );
}

function AudioCard(props: { s: LocalAvState; path: string }): JSX.Element {
    const s = props.s;
    return (
        <Show
            when={s.playingUrl()}
            fallback={
                <div class="am-media-audio-card">
                    <PlayButton s={s} path={props.path} />
                    <AvLabel s={s} path={props.path} />
                </div>
            }
        >
            {(url) => <audio class="am-media-player" src={url()} controls controlslist={NATIVE_PLAYER_CONTROLS_LIST} autoplay onError={s.onMediaError} />}
        </Show>
    );
}

/** Opens the message's media over the tab, the video first: from where the inline player is, which stops. */
function enlargeVideo(stage: HTMLElement | undefined, path: string, openGallery: (g: MediaGallery) => void): void {
    const figure = stage?.closest<HTMLElement>("figure[data-am-media-path]");
    if (!figure) return;
    const gallery = messageGallery(figure);
    if (gallery.index < 0) return;
    const inline = inlineVideos.get(path);
    // Not played yet: the enlarged player starts at the beginning, muted like the inline one, waiting for a press.
    let start: VideoHandoff = { path, time: 0, muted: true, paused: true };
    if (inline?.isConnected) {
        start = { path, time: inline.currentTime, muted: inline.muted, paused: inline.paused };
        // Two players never make sound at once.
        inline.pause();
    }
    openGallery({ ...gallery, start });
}

function VideoStage(props: { s: LocalAvState; path: string; openGallery?: MarkdownMediaOpts["openGallery"] }): JSX.Element {
    const s = props.s;
    let stage: HTMLDivElement | undefined;
    const canEnlarge = () => !!props.openGallery && ["ready", "loading", "playing"].includes(s.state().kind);
    return (
        <div class="am-media-stage" ref={stage}>
            <Show when={canEnlarge()}>
                <IconButton
                    class="am-media-enlarge"
                    icon="expand"
                    label={t("mediaPreview.enlarge", { name: basenameOf(props.path) })}
                    tooltip={false}
                    {...tip("Enlarge")}
                    onClick={() => enlargeVideo(stage, props.path, props.openGallery!)}
                />
            </Show>
            <Show
                when={s.playingUrl()}
                fallback={
                    <>
                        <Show when={s.poster() && !s.posterFailed()}>
                            <video
                                class="am-media-poster"
                                // `#t=0.1`: seek past a black first frame; `auto`:
                                // the bytes are already in memory, so decode one.
                                src={`${s.poster()}#t=0.1`}
                                preload="auto"
                                muted
                                playsinline
                                onError={() => s.setPosterFailed(true)}
                            />
                        </Show>
                        <div class="am-media-stage-overlay">
                            <PlayButton s={s} path={props.path} />
                            <AvLabel s={s} path={props.path} />
                        </div>
                    </>
                }
            >
                {(url) => {
                    let el: HTMLVideoElement | undefined;
                    onCleanup(() => {
                        if (el && inlineVideos.get(props.path) === el) inlineVideos.delete(props.path);
                    });
                    return (
                        <video
                            // Starts muted (§5); the property, not just
                            // the attribute, so it holds whatever loads.
                            ref={(v) => {
                                el = v;
                                v.muted = true;
                                inlineVideos.set(props.path, v);
                            }}
                            class="am-media-player"
                            src={url()}
                            controls
                            controlslist={NATIVE_PLAYER_CONTROLS_LIST}
                            autoplay
                            muted
                            playsinline
                            onError={s.onMediaError}
                        />
                    );
                }}
            </Show>
        </div>
    );
}

/** The player itself: a video's reserved stage, or an audio file's card. */
export function AvFigure(props: {
    s: LocalAvState;
    path: string;
    kind: "video" | "audio";
    openGallery?: MarkdownMediaOpts["openGallery"];
}): JSX.Element {
    return (
        <figure
            class={`am-media am-media-${props.kind}`}
            ref={props.s.setBox}
            // Anywhere on it, before play (the poster and the play button) and after: the same items as the media
            // viewer. The element is read when Copy frame is pressed: the poster first, the player once it plays.
            onContextMenu={(e) => {
                const figure = e.currentTarget;
                showMediaCopyMenu(e, { kind: props.kind, mime: "", path: props.path, element: () => figure.querySelector("video") });
            }}
            // A video of an agent's message is one of the preview's items; audio is not.
            {...(props.kind === "video" ? { "data-am-media-path": props.path, "data-am-media-kind": "video" } : {})}
        >
            <Show when={props.kind === "video"} fallback={<AudioCard s={props.s} path={props.path} />}>
                <VideoStage s={props.s} path={props.path} openGallery={props.openGallery} />
            </Show>
        </figure>
    );
}

/**
 * A local video or audio file (§5). Nothing autoplays on its own. Before play
 * it reads only a small range: a video's first 2 MB for a poster frame (a file
 * whose index is at the end shows a plain placeholder instead), an audio
 * file's first byte for its size. Pressing play fetches the whole file, under
 * the 200 MB cap, and plays it (video muted, with controls). A video keeps its
 * reserved 16:9 stage throughout, so its row never changes height.
 */
function LocalAV(props: {
    path: string;
    src: string;
    kind: "video" | "audio";
    openGallery?: MarkdownMediaOpts["openGallery"];
    owner?: MediaOwner;
}): JSX.Element {
    const s = createLocalAv(props);
    return (
        <Switch fallback={<AvFigure s={s} path={props.path} kind={props.kind} openGallery={props.openGallery} />}>
            <Match when={s.state().kind === "missing"}>
                <span class="am-muted">
                    [{props.kind} not found: {props.src}]{reasonOf(s.state())}
                </span>
            </Match>
            <Match when={s.state().kind === "error"}>
                <span class="am-muted">
                    [{props.kind} can't play: {s.errorState()}]
                </span>
            </Match>
            <Match when={s.state().kind === "too-large"}>
                <MediaCard path={props.path} owner={props.owner} size={s.tooLargeSize()} icon={props.kind === "video" ? "fa-film" : "fa-music"} />
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
                <Button class="am-media-chip" onClick={() => setLoad(true)} title={props.src}>
                    🌐 image from {host} — load{insecure ? " (http, not encrypted)" : ""}
                </Button>
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
    // Fail closed: a message of an agent not known to run on this computer, with no pane to name, reads nothing. srv
    // decides by the pane's runtime; this flag only picks the route, and without a pane there is no route.
    if (p.media.confined && !p.media.blockId) return <span class="am-muted">{t("mediaPreview.notShownHere", { name: shown })}</span>;
    const owner: MediaOwner = { blockId: p.media.blockId, confined: p.media.confined };
    if (kind === "image") return <LocalImage path={path} src={shown} alt={alt} openGallery={p.media.openGallery} owner={owner} />;
    if (kind === "video" || kind === "audio") return <LocalAV path={path} src={shown} kind={kind} openGallery={p.media.openGallery} owner={owner} />;
    return <span class="am-muted">[image: {shown} — unsupported type]</span>;
};

export { MarkdownImg, MuxBlock };
