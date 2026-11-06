// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The PDF viewer's state: the signals it owns and the fields that carry pdf.js's objects and the load in
 * progress from one step to the next. Created first in the component; the hook groups that follow add the
 * effects in their original order.
 */

import { createSignal } from "solid-js";

import { acquireViewerSignal } from "./pdf-viewer-signal";
import type {
    FindMatches,
    PdfDocument,
    PdfEventBus,
    PdfLoadingTask,
    PdfViewerInstance,
    PdfViewerProps,
    Status,
} from "./pdf-viewer-model";
import type { PdfjsModules } from "./pdfjs-runtime";

export function createPdfViewerState(props: PdfViewerProps) {
    const [status, setStatus] = createSignal<Status>("loading");
    const [error, setError] = createSignal("");
    const [page, setPage] = createSignal(1);
    const [pages, setPages] = createSignal(0);
    const [scale, setScale] = createSignal(1);
    const [query, setQuery] = createSignal("");
    const [matches, setMatches] = createSignal<FindMatches | null>(null);
    const [wrongPassword, setWrongPassword] = createSignal(false);
    const [invert, setInvert] = createSignal(false);

    return {
        props,
        status,
        setStatus,
        error,
        setError,
        page,
        setPage,
        pages,
        setPages,
        scale,
        setScale,
        query,
        setQuery,
        matches,
        setMatches,
        wrongPassword,
        setWrongPassword,
        invert,
        setInvert,
        // Set by the view's refs before anything reads them.
        container: undefined as unknown as HTMLDivElement,
        viewerEl: undefined as unknown as HTMLDivElement,
        findInput: undefined as HTMLInputElement | undefined,
        pageInput: undefined as HTMLInputElement | undefined,
        passwordInput: undefined as HTMLInputElement | undefined,
        mods: null as PdfjsModules | null,
        viewer: null as PdfViewerInstance | null,
        eventBus: null as PdfEventBus | null,
        loadingTask: null as PdfLoadingTask | null,
        doc: null as PdfDocument | null,
        sendPassword: null as ((password: string) => void) | null,
        generation: 0,
        // pdf.js's viewer drops its resize observer and scroll listeners only
        // when its signal aborts; setDocument(null) leaves them. Shared by every
        // PDF viewer: see pdf-viewer-signal.ts.
        viewerSignal: acquireViewerSignal(),
        // The file read in progress, stopped when the viewer moves on.
        readAbort: null as AbortController | null,
        // A reload (the file changed on disk) keeps the page; the zoom is the pane's.
        keep: null as { page: number } | null,
        // The scale that fits the width of the pane, which the pane's zoom is a multiple of.
        fitWidth: 0,
    };
}

export type PdfViewerState = ReturnType<typeof createPdfViewerState>;
