// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's image viewer: moved from the retired Media pane with
 * its zoom and pan unchanged — the wheel zooms about the cursor (Ctrl+wheel
 * stays pane zoom), a drag pans, double-click toggles fit and actual size,
 * `+ - 0 1` and the arrows, pixel squares past 2×, the zoom kept across a
 * same-size re-render. Added: a toolbar (fit, 100%, background), the image's
 * size and type, and `[` / `]` for the previous and next image in the folder.
 * SVG is only ever an `<img>` from a blob, so its scripts never run.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.2, §4.13.
 */

import { formatBytes } from "@/app/element/local-media";
import { Button, IconButton } from "@/app/element/ui";
import { DURATION_QUICK_MS, EASE_STANDARD } from "@/app/motion/tokens";
import { createEffect, createSignal, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import { createFileBytes } from "../file-bytes";
import { ViewerBar, type ViewerBarProps } from "../viewer-bar";
import {
    actualSizeScale,
    clampPan,
    FIT,
    panBy,
    showsPixels,
    toggleZoom,
    wheelFactor,
    zoomAt,
    zoomPercent,
    type ZoomBox,
    type ZoomState,
} from "./image-zoom";
import "./image-viewer.scss";

export type ImageBackground = "dark" | "light" | "checker";

const BACKGROUNDS: ImageBackground[] = ["checker", "dark", "light"];

/** The background an image starts on: a checkerboard for formats that can be
 *  transparent, the pane's own for those that can't. */
export function defaultBackground(mime: string): ImageBackground {
    return /jpeg|bmp/.test(mime) ? "dark" : "checker";
}

export interface ImageViewerProps {
    /** The file ("" for dropped bytes). */
    path: string;
    file?: File;
    revision: number;
    /** From the probe, or the dropped file's type. */
    mime: string;
    size?: number;
    bar: Omit<ViewerBarProps, "start" | "facts">;
    /** `[` / `]`: the previous or next image in the folder. */
    onStep?: (delta: -1 | 1) => void;
}

export function ImageViewer(props: ImageViewerProps): JSX.Element {
    const isSvg = () => props.mime === "image/svg+xml" || /\.svg$/i.test(props.path || props.file?.name || "");
    const bytes = createFileBytes(() => ({
        path: props.path,
        file: props.file,
        revision: props.revision,
        type: isSvg() ? "image/svg+xml" : undefined,
    }));

    const [ready, setReady] = createSignal(false);
    const [zoom, setZoom] = createSignal<ZoomState>(FIT);
    const [dragging, setDragging] = createSignal(false);
    const [natural, setNatural] = createSignal<{ w: number; h: number } | null>(null);
    const [background, setBackground] = createSignal<ImageBackground>(defaultBackground(props.mime));
    // A new image fades in once it has decoded.
    createEffect(on(bytes.url, () => setReady(false), { defer: true }));
    let contentRef: HTMLDivElement | undefined;
    let imgRef: HTMLImageElement | undefined;
    // The last image's own size: a newer render of the same size keeps the
    // zoom and pan (comparing renders), another size goes back to fit.
    let lastNatural = "";

    const zoomBox = (): ZoomBox | null => {
        if (!contentRef || !imgRef || !imgRef.naturalWidth) return null;
        return {
            fitWidth: imgRef.offsetWidth,
            fitHeight: imgRef.offsetHeight,
            viewWidth: contentRef.offsetWidth,
            viewHeight: contentRef.offsetHeight,
            naturalWidth: imgRef.naturalWidth,
            naturalHeight: imgRef.naturalHeight,
        };
    };
    // A point on screen, in the view's own CSS pixels from its centre (the
    // pane may be zoomed, which scales screen pixels against CSS pixels).
    const fromCentre = (clientX: number, clientY: number): { x: number; y: number } => {
        const rect = contentRef!.getBoundingClientRect();
        const k = contentRef!.offsetWidth ? rect.width / contentRef!.offsetWidth : 1;
        return {
            x: (clientX - rect.left) / k - contentRef!.offsetWidth / 2,
            y: (clientY - rect.top) / k - contentRef!.offsetHeight / 2,
        };
    };
    const screenScale = (): number => {
        const rect = contentRef?.getBoundingClientRect();
        return rect && contentRef!.offsetWidth ? rect.width / contentRef!.offsetWidth : 1;
    };
    const centre = { x: 0, y: 0 };
    const showsImage = (): boolean => !bytes.error() && !!bytes.url() && ready();

    const onWheel = (e: WheelEvent) => {
        // Ctrl/Cmd+wheel zooms the pane itself (app.tsx), as everywhere.
        if (e.ctrlKey || e.metaKey || !showsImage()) return;
        const box = zoomBox();
        if (!box) return;
        e.preventDefault();
        setZoom(zoomAt(zoom(), wheelFactor(e.deltaY, e.deltaMode), fromCentre(e.clientX, e.clientY), box));
    };

    let dragFrom: { x: number; y: number } | null = null;
    const onPointerDown = (e: PointerEvent) => {
        if (e.button !== 0 || zoom().scale <= 1) return;
        e.preventDefault();
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        dragFrom = { x: e.clientX, y: e.clientY };
        setDragging(true);
    };
    const onPointerMove = (e: PointerEvent) => {
        const box = zoomBox();
        if (!dragFrom || !box) return;
        const k = screenScale();
        setZoom(panBy(zoom(), (e.clientX - dragFrom.x) / k, (e.clientY - dragFrom.y) / k, box));
        dragFrom = { x: e.clientX, y: e.clientY };
    };
    const endDrag = () => {
        dragFrom = null;
        setDragging(false);
    };
    const onDoubleClick = (e: MouseEvent) => {
        const box = zoomBox();
        if (box) setZoom(toggleZoom(zoom(), fromCentre(e.clientX, e.clientY), box));
    };
    const fit = () => setZoom(FIT);
    const actualSize = () => {
        const box = zoomBox();
        if (box) setZoom(zoomAt(zoom(), actualSizeScale(box) / zoom().scale, centre, box));
    };
    // + / - zoom about the centre, 0 fits, 1 shows actual size; zoomed in,
    // the arrow keys pan a tenth of the view. [ and ] step through the folder.
    const onKeyDown = (e: KeyboardEvent) => {
        if (e.ctrlKey || e.metaKey || e.altKey) return;
        if ((e.key === "[" || e.key === "]") && props.onStep) {
            e.preventDefault();
            e.stopPropagation();
            props.onStep(e.key === "[" ? -1 : 1);
            return;
        }
        if (e.key === "b" || e.key === "B") {
            e.preventDefault();
            cycleBackground();
            return;
        }
        if (!showsImage()) return;
        const box = zoomBox();
        if (!box) return;
        let next: ZoomState | null = null;
        if (e.key === "+" || e.key === "=") next = zoomAt(zoom(), 1.25, centre, box);
        else if (e.key === "-" || e.key === "_") next = zoomAt(zoom(), 1 / 1.25, centre, box);
        else if (e.key === "0") next = FIT;
        else if (e.key === "1") next = zoomAt(zoom(), actualSizeScale(box) / zoom().scale, centre, box);
        else if (zoom().scale > 1 && e.key.startsWith("Arrow")) {
            const step = { x: box.viewWidth / 10, y: box.viewHeight / 10 };
            const d = { ArrowLeft: [step.x, 0], ArrowRight: [-step.x, 0], ArrowUp: [0, step.y], ArrowDown: [0, -step.y] }[e.key];
            if (d) next = panBy(zoom(), d[0], d[1], box);
        }
        if (!next) return;
        e.preventDefault();
        e.stopPropagation();
        setZoom(next);
    };
    const onImageLoad = () => {
        const n = imgRef ? `${imgRef.naturalWidth}x${imgRef.naturalHeight}` : "";
        if (n !== lastNatural) setZoom(FIT);
        lastNatural = n;
        if (imgRef) setNatural({ w: imgRef.naturalWidth, h: imgRef.naturalHeight });
        setReady(true);
    };
    const cycleBackground = () => {
        const i = BACKGROUNDS.indexOf(background());
        setBackground(BACKGROUNDS[(i + 1) % BACKGROUNDS.length]);
    };

    onMount(() => {
        // A real listener, not passive, so it can keep the wheel from scrolling.
        contentRef?.addEventListener("wheel", onWheel, { passive: false });
        onCleanup(() => contentRef?.removeEventListener("wheel", onWheel));
        // A resized pane keeps the image in reach.
        if (typeof ResizeObserver !== "undefined" && contentRef) {
            const resize = new ResizeObserver(() => {
                const box = zoomBox();
                if (box && zoom().scale > 1) setZoom(clampPan(zoom(), box));
            });
            resize.observe(contentRef);
            onCleanup(() => resize.disconnect());
        }
    });

    const percent = () => {
        const box = zoomBox();
        return box ? `${zoomPercent(zoom(), box)}%` : "";
    };
    const facts = (): string[] => {
        const out: string[] = [];
        const n = natural();
        if (n) out.push(`${n.w} × ${n.h}`);
        const size = props.size ?? props.file?.size;
        if (size != null) out.push(formatBytes(size));
        const type = props.mime.replace(/^image\//, "").replace(/^svg\+xml$/, "svg").replace(/^x-icon$/, "ico").toUpperCase();
        if (type) out.push(type);
        return out;
    };

    return (
        <div class="editor-viewer image-viewer">
            <ViewerBar
                {...props.bar}
                facts={facts()}
                start={
                    <>
                        <Button density="compact" title="Fit to the pane (0)" pressed={zoom().scale === 1} onClick={fit}>
                            Fit
                        </Button>
                        <Button density="compact" title="Actual size (1)" onClick={actualSize}>
                            100%
                        </Button>
                        <Show when={showsImage()}>
                            <span class="image-viewer-percent" aria-label="Zoom">
                                {percent()}
                            </span>
                        </Show>
                        <IconButton
                            icon={background() === "light" ? "sun" : background() === "dark" ? "moon" : "chess-board"}
                            density="compact"
                            label={`Background: ${background()} (B)`}
                            onClick={cycleBackground}
                        />
                    </>
                }
            />
            <div
                ref={contentRef}
                class={`image-viewer-content image-viewer-bg-${background()}`}
                tabIndex={0}
                aria-label="Image"
                onKeyDown={onKeyDown}
            >
                <Show when={bytes.error()}>
                    <div class="editor-viewer-message" role="alert">
                        {bytes.error()}
                    </div>
                </Show>
                <Show when={!bytes.error() && !bytes.url()}>
                    <div class="editor-viewer-message">Loading…</div>
                </Show>
                <Show when={!bytes.error() && bytes.url()}>
                    <img
                        ref={imgRef}
                        class="image-viewer-img"
                        classList={{
                            "image-viewer-zoomed": zoom().scale > 1,
                            "image-viewer-dragging": dragging(),
                            "image-viewer-pixels": (() => {
                                const box = zoomBox();
                                return !!box && showsPixels(zoom(), box);
                            })(),
                        }}
                        style={{
                            opacity: ready() ? 1 : 0,
                            transition: `opacity ${DURATION_QUICK_MS}ms ${EASE_STANDARD}`,
                            transform: `translate(${zoom().x}px, ${zoom().y}px) scale(${zoom().scale})`,
                        }}
                        alt={props.path || props.file?.name || "image"}
                        src={bytes.url()!}
                        draggable={zoom().scale <= 1}
                        onLoad={onImageLoad}
                        onDblClick={onDoubleClick}
                        onPointerDown={onPointerDown}
                        onPointerMove={onPointerMove}
                        onPointerUp={endDrag}
                        onPointerCancel={endDrag}
                        onError={() => bytes.setError("Couldn't display the image (unsupported format or a damaged file).")}
                    />
                </Show>
                <Show when={showsImage() && zoom().scale > 1}>
                    <div class="image-viewer-zoom-badge">
                        <Button density="compact" title="Fit to the pane (0)" onClick={fit}>
                            {percent()} · Fit
                        </Button>
                    </div>
                </Show>
            </div>
        </div>
    );
}
