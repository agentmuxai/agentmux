// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the PDF viewer is given, the constants it works by and the shapes of pdf.js it holds on to. */

import type { ViewerBarProps } from "../viewer-bar";
import type { PdfjsModules } from "./pdfjs-runtime";

/** Files up to this size load whole; larger ones in ranges. */
export const WHOLE_FILE_MAX_BYTES = 32 * 1024 * 1024;
/** pdf.js's default range size. */
export const RANGE_CHUNK = 65536;
/** A zoom step is drawn after this long, so a run of wheel ticks draws the pages once. */
export const REDRAW_DELAY_MS = 150;
/** The step of the toolbar's zoom buttons. */
export const BUTTON_STEP = 0.1;

export interface PdfViewerProps {
    path: string;
    file?: File;
    revision: number;
    size?: number;
    bar: Omit<ViewerBarProps, "start" | "facts">;
    /** An http(s) link was clicked. */
    onOpenLink: (url: string) => void;
    /** The pane this is in, and its zoom: a multiple of the width that fits (pdf-zoom.ts). */
    blockId: string;
    zoom: () => number;
}

export type Status = "loading" | "ready" | "password" | "error";

export type PdfViewerInstance = InstanceType<PdfjsModules["viewer"]["PDFViewer"]>;
export type PdfEventBus = InstanceType<PdfjsModules["viewer"]["EventBus"]>;
export type PdfLoadingTask = ReturnType<PdfjsModules["pdfjs"]["getDocument"]>;
export type PdfDocument = Awaited<PdfLoadingTask["promise"]>;
export type FindMatches = { current: number; total: number };
