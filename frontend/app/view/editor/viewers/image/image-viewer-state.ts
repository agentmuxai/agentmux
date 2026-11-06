// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The image viewer's state (image-viewer.tsx): the bytes, the zoom and pan, the wheel, the press and the keys, the
 * modifier that turns a click into zoom-out, the background and the facts. The pieces are made in the order the
 * viewer always made them.
 */

import { formatBytes } from "@/util/format-bytes";
import { isMacOS } from "@/util/platformutil";
import { createEffect, createSignal, on, onCleanup, onMount, type Accessor, type Setter } from "solid-js";
import { createFileBytes } from "../file-bytes";
import type { ViewerBarProps } from "../viewer-bar";
import {
    actualSizeScale,
    clampPan,
    FIT,
    panBy,
    wheelFactor,
    zoomAt,
    zoomPercent,
    zoomStep,
    type ZoomBox,
    type ZoomState,
} from "./image-zoom";

export type ImageBackground = "dark" | "light" | "checker";

const BACKGROUNDS: ImageBackground[] = ["checker", "dark", "light"];

/** A press that moves less than this many pixels is a click, not a pan. */
const CLICK_SLOP_PX = 4;

/** The modifier that makes a click zoom out: Alt (⌥) everywhere, and Ctrl
 *  too except on macOS, where Ctrl+click is a right-click. */
function isZoomOutModifier(e: { altKey: boolean; ctrlKey: boolean }): boolean {
    return e.altKey || (e.ctrlKey && !isMacOS());
}

/** The background an image starts on: a checkerboard for formats that can be
 *  transparent, the pane's own for those that can't. */
export function defaultBackground(mime: string): ImageBackground {
    return /jpeg|bmp/.test(mime) ? "dark" : "checker";
}

/** Keep a press on the image to the viewer: no native drag, the pointer captured. preventDefault also stops the
 *  browser moving focus here, and the keys (+ - arrows [ ]) need it, so the viewer takes it. */
function holdPress(e: PointerEvent, content: HTMLElement | undefined): void {
    e.preventDefault();
    content?.focus({ preventScroll: true });
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
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

/** The two elements the zoom is measured against, and the points on screen as the view sees them. */
function createGeometry() {
    let contentRef: HTMLDivElement | undefined;
    let imgRef: HTMLImageElement | undefined;

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
    return {
        content: () => contentRef,
        img: () => imgRef,
        setContentRef: (el: HTMLDivElement) => (contentRef = el),
        setImgRef: (el: HTMLImageElement) => (imgRef = el),
        zoomBox,
        fromCentre,
        screenScale,
    };
}
type Geometry = ReturnType<typeof createGeometry>;

interface Zooming {
    zoom: Accessor<ZoomState>;
    setZoom: Setter<ZoomState>;
    geo: Geometry;
    showsImage: () => boolean;
}

/** The wheel, and a press that is a click until it moves, then a pan; the modifier that makes a click zoom out. */
function createPointer({ zoom, setZoom, geo, showsImage }: Zooming) {
    const [dragging, setDragging] = createSignal(false);
    // The zoom-out modifier is down: the cursor turns to zoom-out.
    const [zoomOutHeld, setZoomOutHeld] = createSignal(false);

    const onWheel = (e: WheelEvent) => {
        // Ctrl/Cmd+wheel zooms the pane itself (app.tsx), as everywhere.
        if (e.ctrlKey || e.metaKey || !showsImage()) return;
        const box = geo.zoomBox();
        if (!box) return;
        e.preventDefault();
        setZoom(zoomAt(zoom(), wheelFactor(e.deltaY, e.deltaMode), geo.fromCentre(e.clientX, e.clientY), box));
    };

    // A press is a click until it moves CLICK_SLOP_PX; from then on it pans
    // (when zoomed in), and its release doesn't zoom.
    let press: { x: number; y: number; lastX: number; lastY: number; panning: boolean } | null = null;
    const endPress = () => {
        press = null;
        setDragging(false);
    };
    const onPointerDown = (e: PointerEvent) => {
        setZoomOutHeld(isZoomOutModifier(e));
        // On macOS Ctrl+click is a right-click.
        if (e.button !== 0 || (e.ctrlKey && isMacOS())) return;
        press = { x: e.clientX, y: e.clientY, lastX: e.clientX, lastY: e.clientY, panning: false };
        // No native drag of the image: dropped on the app it opened another copy of the file.
        holdPress(e, geo.content());
    };
    const onPointerMove = (e: PointerEvent) => {
        setZoomOutHeld(isZoomOutModifier(e));
        if (!press) return;
        // Released outside the image, with nothing capturing it.
        if (e.buttons === 0) return endPress();
        if (!press.panning && Math.hypot(e.clientX - press.x, e.clientY - press.y) < CLICK_SLOP_PX) return;
        press.panning = true;
        const box = geo.zoomBox();
        if (!box || zoom().scale <= 1) return;
        setDragging(true);
        const k = geo.screenScale();
        setZoom(panBy(zoom(), (e.clientX - press.lastX) / k, (e.clientY - press.lastY) / k, box));
        press.lastX = e.clientX;
        press.lastY = e.clientY;
    };
    const onPointerUp = (e: PointerEvent) => {
        const click = press && !press.panning;
        endPress();
        const box = geo.zoomBox();
        if (!click || !box || !showsImage()) return;
        setZoom(zoomStep(zoom(), isZoomOutModifier(e) ? -1 : 1, geo.fromCentre(e.clientX, e.clientY), box));
    };
    return { dragging, zoomOutHeld, setZoomOutHeld, onWheel, onPointerDown, onPointerMove, onPointerUp, endPress };
}

const centre = { x: 0, y: 0 };

/** + / - zoom about the centre, 0 fits, 1 shows actual size; zoomed in, the arrow keys pan a tenth of the view. [ and ]
 *  step through the folder, and B cycles the background. */
function createKeys(props: ImageViewerProps, z: Zooming, cycleBackground: () => void) {
    const { zoom, setZoom, geo, showsImage } = z;
    return (e: KeyboardEvent) => {
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
        const box = geo.zoomBox();
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
}

/** The wheel listener, the modifier listeners on the window, and the observer that keeps a resized pane's image in reach. */
function listenWhileMounted(
    { zoom, setZoom, geo }: Zooming,
    onWheel: (e: WheelEvent) => void,
    setZoomOutHeld: Setter<boolean>,
) {
    onMount(() => {
        const contentRef = geo.content();
        // A real listener, not passive, so it can keep the wheel from scrolling.
        contentRef?.addEventListener("wheel", onWheel, { passive: false });
        onCleanup(() => contentRef?.removeEventListener("wheel", onWheel));
        // The zoom-out cursor follows the modifier even while the pointer
        // rests, so it listens on the window, not only the image.
        const onModifier = (e: KeyboardEvent) => setZoomOutHeld(isZoomOutModifier(e));
        const onBlur = () => setZoomOutHeld(false);
        window.addEventListener("keydown", onModifier);
        window.addEventListener("keyup", onModifier);
        window.addEventListener("blur", onBlur);
        onCleanup(() => {
            window.removeEventListener("keydown", onModifier);
            window.removeEventListener("keyup", onModifier);
            window.removeEventListener("blur", onBlur);
        });
        // A resized pane keeps the image in reach.
        if (typeof ResizeObserver !== "undefined" && contentRef) {
            const resize = new ResizeObserver(() => {
                const box = geo.zoomBox();
                if (box && zoom().scale > 1) setZoom(clampPan(zoom(), box));
            });
            resize.observe(contentRef);
            onCleanup(() => resize.disconnect());
        }
    });
}

/** The image's size in pixels, its byte size and its type. */
function imageFacts(props: ImageViewerProps, natural: Accessor<{ w: number; h: number } | null>): string[] {
    const out: string[] = [];
    const n = natural();
    if (n) out.push(`${n.w} × ${n.h}`);
    const size = props.size ?? props.file?.size;
    if (size != null) out.push(formatBytes(size));
    const type = props.mime.replace(/^image\//, "").replace(/^svg\+xml$/, "svg").replace(/^x-icon$/, "ico").toUpperCase();
    if (type) out.push(type);
    return out;
}

/** The image viewer's state: everything its markup reads, and what its controls do. */
export function createImageViewer(props: ImageViewerProps) {
    const isSvg = () => props.mime === "image/svg+xml" || /\.svg$/i.test(props.path || props.file?.name || "");
    const bytes = createFileBytes(() => ({
        path: props.path,
        file: props.file,
        revision: props.revision,
        type: isSvg() ? "image/svg+xml" : undefined,
    }));

    const [ready, setReady] = createSignal(false);
    const [zoom, setZoom] = createSignal<ZoomState>(FIT);
    const [natural, setNatural] = createSignal<{ w: number; h: number } | null>(null);
    const [background, setBackground] = createSignal<ImageBackground>(defaultBackground(props.mime));
    // A new image fades in once it has decoded.
    createEffect(on(bytes.url, () => setReady(false), { defer: true }));
    // The last image's own size: a newer render of the same size keeps the
    // zoom and pan (comparing renders), another size goes back to fit.
    let lastNatural = "";

    const geo = createGeometry();
    const showsImage = (): boolean => !bytes.error() && !!bytes.url() && ready();
    const z: Zooming = { zoom, setZoom, geo, showsImage };
    const pointer = createPointer(z);

    const fit = () => setZoom(FIT);
    const actualSize = () => {
        const box = geo.zoomBox();
        if (box) setZoom(zoomAt(zoom(), actualSizeScale(box) / zoom().scale, centre, box));
    };
    const cycleBackground = () => {
        const i = BACKGROUNDS.indexOf(background());
        setBackground(BACKGROUNDS[(i + 1) % BACKGROUNDS.length]);
    };
    const onKeyDown = createKeys(props, z, cycleBackground);
    const onImageLoad = () => {
        const img = geo.img();
        const n = img ? `${img.naturalWidth}x${img.naturalHeight}` : "";
        if (n !== lastNatural) setZoom(FIT);
        lastNatural = n;
        if (img) setNatural({ w: img.naturalWidth, h: img.naturalHeight });
        setReady(true);
    };

    listenWhileMounted(z, pointer.onWheel, pointer.setZoomOutHeld);

    const percent = () => {
        const box = geo.zoomBox();
        return box ? `${zoomPercent(zoom(), box)}%` : "";
    };
    const facts = () => imageFacts(props, natural);

    return { bytes, ready, zoom, background, geo, showsImage, ...pointer, fit, actualSize, cycleBackground, onKeyDown, onImageLoad, percent, facts };
}

/** What the image viewer's markup reads. */
export type ImageViewerState = ReturnType<typeof createImageViewer>;
