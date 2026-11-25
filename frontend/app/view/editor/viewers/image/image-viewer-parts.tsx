// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The image viewer's markup in parts (image-viewer.tsx): the toolbar's buttons and the stage with the image, its
 * messages and the zoom badge. Each reads the viewer's state (image-viewer-state.ts).
 */

import { Button, IconButton } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { EASE_STANDARD } from "@/app/motion/tokens";
import { Show, type JSX } from "solid-js";
import { atMaxZoom, showsPixels } from "./image-zoom";
import type { ImageViewerProps, ImageViewerState } from "./image-viewer-state";

type Part = { t: ImageViewerState };

/** Fit, 100%, the zoom shown as a percent, and the background. */
export function ImageTools(props: Part): JSX.Element {
    const v = props.t;
    return (
        <>
            <Button density="compact" title={t("imageViewer.fitTip")} pressed={v.zoom().scale === 1} onClick={v.fit}>
                {t("imageViewer.fit")}
            </Button>
            <Button density="compact" title={t("imageViewer.actualTip")} onClick={v.actualSize}>
                100%
            </Button>
            <Show when={v.showsImage()}>
                <span class="image-viewer-percent" aria-label={t("imageViewer.zoom")}>
                    {v.percent()}
                </span>
            </Show>
            <IconButton
                icon={v.background() === "light" ? "sun" : v.background() === "dark" ? "moon" : "chess-board"}
                density="compact"
                label={t("imageViewer.background", { mode: v.background() })}
                onClick={v.cycleBackground}
            />
        </>
    );
}

/** The image itself, which a press zooms or pans. */
function ZoomableImage(props: Part & { alt: string }): JSX.Element {
    const v = props.t;
    return (
        <img
            ref={v.geo.setImgRef}
            class="image-viewer-img"
            classList={{
                "image-viewer-deepest": (() => {
                    const box = v.geo.zoomBox();
                    return !!box && atMaxZoom(v.zoom(), box);
                })(),
                "image-viewer-zoom-out": v.zoomOutHeld(),
                "image-viewer-dragging": v.dragging(),
                "image-viewer-pixels": (() => {
                    const box = v.geo.zoomBox();
                    return !!box && showsPixels(v.zoom(), box);
                })(),
            }}
            style={{
                opacity: v.ready() ? 1 : 0,
                transition: `opacity var(--duration-quick) ${EASE_STANDARD}`,
                transform: `translate(${v.zoom().x}px, ${v.zoom().y}px) scale(${v.zoom().scale})`,
            }}
            alt={props.alt}
            src={v.bytes.url()!}
            draggable={false}
            onDragStart={(e) => e.preventDefault()}
            onLoad={v.onImageLoad}
            onPointerDown={v.onPointerDown}
            onPointerMove={v.onPointerMove}
            onPointerUp={v.onPointerUp}
            onPointerCancel={v.endPress}
            onError={() => v.bytes.setError(t("imageViewer.cantDisplay"))}
        />
    );
}

/** The view the image sits in: why there is none, the image, and the Fit badge once zoomed in. */
export function ImageStage(props: Part & { source: ImageViewerProps }): JSX.Element {
    const v = props.t;
    return (
        <div
            ref={v.geo.setContentRef}
            class={`image-viewer-content image-viewer-bg-${v.background()}`}
            tabIndex={0}
            aria-label={t("imageViewer.image")}
            onKeyDown={v.onKeyDown}
        >
            <Show when={v.bytes.error()}>
                <div class="editor-viewer-message" role="alert">
                    {v.bytes.error()}
                </div>
            </Show>
            <Show when={!v.bytes.error() && !v.bytes.url()}>
                <div class="editor-viewer-message">{t("imageViewer.loading")}</div>
            </Show>
            <Show when={!v.bytes.error() && v.bytes.url()}>
                <ZoomableImage t={v} alt={props.source.path || props.source.file?.name || t("imageViewer.imageAlt")} />
            </Show>
            <Show when={v.showsImage() && v.zoom().scale > 1}>
                <div class="image-viewer-zoom-badge">
                    <Button density="compact" title={t("imageViewer.fitTip")} onClick={v.fit}>
                        {t("imageViewer.fitBadge", { percent: v.percent() })}
                    </Button>
                </div>
            </Show>
        </div>
    );
}
