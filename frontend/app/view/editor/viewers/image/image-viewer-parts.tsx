// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The image viewer's markup in parts (image-viewer.tsx): the toolbar's buttons and the stage with the image, its
 * messages and the zoom badge. Each reads the viewer's state (image-viewer-state.ts).
 */

import { Button, IconButton } from "@/app/element/ui";
import { EASE_STANDARD } from "@/app/motion/tokens";
import { Show, type JSX } from "solid-js";
import { atMaxZoom, showsPixels } from "./image-zoom";
import type { ImageViewerProps, ImageViewerState } from "./image-viewer-state";

type Part = { t: ImageViewerState };

/** Fit, 100%, the zoom shown as a percent, and the background. */
export function ImageTools(props: Part): JSX.Element {
    const t = props.t;
    return (
        <>
            <Button density="compact" title="Fit to the pane (0)" pressed={t.zoom().scale === 1} onClick={t.fit}>
                Fit
            </Button>
            <Button density="compact" title="Actual size (1)" onClick={t.actualSize}>
                100%
            </Button>
            <Show when={t.showsImage()}>
                <span class="image-viewer-percent" aria-label="Zoom">
                    {t.percent()}
                </span>
            </Show>
            <IconButton
                icon={t.background() === "light" ? "sun" : t.background() === "dark" ? "moon" : "chess-board"}
                density="compact"
                label={`Background: ${t.background()} (B)`}
                onClick={t.cycleBackground}
            />
        </>
    );
}

/** The image itself, which a press zooms or pans. */
function ZoomableImage(props: Part & { alt: string }): JSX.Element {
    const t = props.t;
    return (
        <img
            ref={t.geo.setImgRef}
            class="image-viewer-img"
            classList={{
                "image-viewer-deepest": (() => {
                    const box = t.geo.zoomBox();
                    return !!box && atMaxZoom(t.zoom(), box);
                })(),
                "image-viewer-zoom-out": t.zoomOutHeld(),
                "image-viewer-dragging": t.dragging(),
                "image-viewer-pixels": (() => {
                    const box = t.geo.zoomBox();
                    return !!box && showsPixels(t.zoom(), box);
                })(),
            }}
            style={{
                opacity: t.ready() ? 1 : 0,
                transition: `opacity var(--duration-quick) ${EASE_STANDARD}`,
                transform: `translate(${t.zoom().x}px, ${t.zoom().y}px) scale(${t.zoom().scale})`,
            }}
            alt={props.alt}
            src={t.bytes.url()!}
            draggable={false}
            onDragStart={(e) => e.preventDefault()}
            onLoad={t.onImageLoad}
            onPointerDown={t.onPointerDown}
            onPointerMove={t.onPointerMove}
            onPointerUp={t.onPointerUp}
            onPointerCancel={t.endPress}
            onError={() => t.bytes.setError("Couldn't display the image (unsupported format or a damaged file).")}
        />
    );
}

/** The view the image sits in: why there is none, the image, and the Fit badge once zoomed in. */
export function ImageStage(props: Part & { source: ImageViewerProps }): JSX.Element {
    const t = props.t;
    return (
        <div
            ref={t.geo.setContentRef}
            class={`image-viewer-content image-viewer-bg-${t.background()}`}
            tabIndex={0}
            aria-label="Image"
            onKeyDown={t.onKeyDown}
        >
            <Show when={t.bytes.error()}>
                <div class="editor-viewer-message" role="alert">
                    {t.bytes.error()}
                </div>
            </Show>
            <Show when={!t.bytes.error() && !t.bytes.url()}>
                <div class="editor-viewer-message">Loading…</div>
            </Show>
            <Show when={!t.bytes.error() && t.bytes.url()}>
                <ZoomableImage t={t} alt={props.source.path || props.source.file?.name || "image"} />
            </Show>
            <Show when={t.showsImage() && t.zoom().scale > 1}>
                <div class="image-viewer-zoom-badge">
                    <Button density="compact" title="Fit to the pane (0)" onClick={t.fit}>
                        {t.percent()} · Fit
                    </Button>
                </div>
            </Show>
        </div>
    );
}
