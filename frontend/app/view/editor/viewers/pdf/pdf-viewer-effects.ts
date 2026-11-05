// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The PDF viewer's effects, in two groups created in the component's original order: loading (the link click
 * listener and the first open, a reload when the file or the tab changes, the cleanup) and zoom (the pane's zoom
 * and the Editor's zoom anchor).
 */

import { chromeZoomAtom } from "@/app/store/chrome-zoom";
import { setZoomStepRatio } from "@/app/store/zoom";
import { setZoomMax } from "@/app/store/zoom-factor";
import { createEffect, on, onCleanup, onMount } from "solid-js";
import { registerContentZoom } from "../content-zoom";
import { onLinkClick } from "./pdf-viewer-actions";
import { open, teardown } from "./pdf-viewer-load";
import { REDRAW_DELAY_MS } from "./pdf-viewer-model";
import type { PdfViewerState } from "./pdf-viewer-state";
import { showZoom } from "./pdf-viewer-zoom";
import { PDF_ZOOM_MAX, PDF_ZOOM_STEP_RATIO, pdfZoomAnchor } from "./pdf-zoom";

export function usePdfLoading(s: PdfViewerState) {
    onMount(() => {
        const onClick = (e: MouseEvent) => onLinkClick(s, e);
        s.container.addEventListener("click", onClick, true);
        onCleanup(() => s.container.removeEventListener("click", onClick, true));
        void open(s);
    });
    // The file changed on disk, or the tab shows another one.
    createEffect(on(() => [s.props.path, s.props.revision, s.props.file] as const, () => void open(s), { defer: true }));
    onCleanup(() => {
        s.generation++;
        teardown(s);
        s.viewerSignal.release();
        s.viewer = null;
        s.eventBus = null;
    });
}

export function usePdfZoom(s: PdfViewerState) {
    createEffect(on(() => s.props.zoom(), () => showZoom(s, REDRAW_DELAY_MS), { defer: true }));
    // The Editor's zoom anchor holds what is under the pointer through a zoom: the pages, not CodeMirror.
    onMount(() => {
        const release = registerContentZoom(s.props.blockId, {
            scroller: () => s.container,
            adapter: pdfZoomAnchor({ container: () => s.container, chromeZoom: chromeZoomAtom }),
        });
        onCleanup(release);
        onCleanup(setZoomStepRatio(s.props.blockId, PDF_ZOOM_STEP_RATIO));
        onCleanup(setZoomMax(s.props.blockId, PDF_ZOOM_MAX));
    });
}
