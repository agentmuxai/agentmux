// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The PDF viewer's zoom: the pane's zoom is a multiple of the scale that fits the width; the toolbar's buttons move it. */

import { zoomBlockIn, zoomBlockOut, zoomBlockTo } from "@/app/store/zoom";
import { clampZoom } from "@/app/store/zoom-factor";
import { BUTTON_STEP } from "./pdf-viewer-model";
import type { PdfViewerState } from "./pdf-viewer-state";
import { scaleFactorTo, scaleFor, zoomForScale } from "./pdf-zoom";

/** Show the pane's zoom: a multiple of the width that fits. */
export function showZoom(s: PdfViewerState, delay?: number) {
    if (!s.viewer || !s.fitWidth) return;
    const factor = scaleFactorTo(s.viewer.currentScale, scaleFor(s.fitWidth, s.props.zoom()));
    if (factor != null) s.viewer.updateScale({ scaleFactor: factor, drawingDelay: delay });
}

/** Fit the width and take that scale as the pane's 100%, then show the pane's zoom. */
export function fitToWidth(s: PdfViewerState) {
    if (!s.viewer) return;
    s.viewer.currentScaleValue = "page-width";
    s.fitWidth = s.viewer.currentScale;
    showZoom(s);
}

// The buttons are the pane's zoom, as Ctrl+wheel and Ctrl +/- are: one level, saved with the pane.
export function zoomBy(s: PdfViewerState, how: "in" | "out" | "page-width" | "page-fit") {
    const blockId = s.props.blockId;
    if (!s.viewer) return;
    if (how === "in") zoomBlockIn(blockId, BUTTON_STEP);
    else if (how === "out") zoomBlockOut(blockId, BUTTON_STEP);
    else if (how === "page-width") {
        fitToWidth(s);
        zoomBlockTo(blockId, 1);
    } else {
        s.viewer.currentScaleValue = "page-width";
        s.fitWidth = s.viewer.currentScale;
        s.viewer.currentScaleValue = "page-fit";
        zoomBlockTo(blockId, clampZoom(zoomForScale(s.viewer.currentScale, s.fitWidth), blockId));
    }
}
