// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Opening a document in the PDF viewer: where its bytes come from (the whole file, or a range transport for a
 * large one), the page viewer pdf.js builds, and the password step. Every step checks the generation it started
 * in, so a load that was superseded (another file, a reload) drops its result.
 */

import { fetchMediaBlob, fetchMediaRange } from "@/app/element/local-media";
import { loadRange } from "./pdf-range";
import { RANGE_CHUNK, WHOLE_FILE_MAX_BYTES, type FindMatches } from "./pdf-viewer-model";
import type { PdfViewerState } from "./pdf-viewer-state";
import { fitToWidth } from "./pdf-viewer-zoom";
import type { PdfjsModules } from "./pdfjs-runtime";

export function teardown(s: PdfViewerState) {
    s.sendPassword = null;
    s.readAbort?.abort();
    s.readAbort = null;
    void s.loadingTask?.destroy().catch(() => {});
    s.loadingTask = null;
    // Destroying the loading task destroys its document and its worker.
    s.viewer?.setDocument(null as never);
    s.doc = null;
}

/** The document source: the whole file, or a range transport. */
async function source(s: PdfViewerState, pdfjs: PdfjsModules["pdfjs"]) {
    const props = s.props;
    if (props.file) return { data: new Uint8Array(await props.file.arrayBuffer()) };
    s.readAbort = new AbortController();
    const signal = s.readAbort.signal;
    if ((props.size ?? 0) <= WHOLE_FILE_MAX_BYTES) {
        return { data: new Uint8Array(await (await fetchMediaBlob(props.path, { signal })).arrayBuffer()) };
    }
    const first = await fetchMediaRange(props.path, 0, RANGE_CHUNK - 1, { signal });
    if (!first.blob) return { data: new Uint8Array(await (await fetchMediaBlob(props.path, { signal })).arrayBuffer()) };
    const range = new pdfjs.PDFDataRangeTransport(first.total, new Uint8Array(await first.blob.arrayBuffer()));
    const path = props.path;
    const mine = s.generation;
    range.requestDataRange = (begin: number, end: number) =>
        void loadRange(
            (b, e) => fetchMediaRange(path, b, e, { signal }),
            begin,
            end,
            (b, bytes) => range.onDataRange(b, bytes),
            (message) => {
                if (mine !== s.generation) return;
                teardown(s);
                s.setError(message);
                s.setStatus("error");
            }
        );
    return { range, length: first.total, rangeChunkSize: RANGE_CHUNK, disableAutoFetch: true };
}

/** The page viewer pdf.js builds on first use, and the events it reports through. */
function createPageViewer(s: PdfViewerState, mods: PdfjsModules) {
    const { pdfjs, viewer: V } = mods;
    const eventBus = new V.EventBus();
    s.eventBus = eventBus;
    const linkService = new V.PDFLinkService({ eventBus });
    const findController = new V.PDFFindController({ eventBus, linkService });
    const viewer = new V.PDFViewer({
        container: s.container,
        viewer: s.viewerEl,
        eventBus,
        linkService,
        findController,
        textLayerMode: 1,
        annotationMode: pdfjs.AnnotationMode.ENABLE,
        // A huge page can't allocate gigabytes of canvas.
        maxCanvasPixels: 2 ** 25,
        // Read by the constructor, missing from 6.4.299's types.
        ...({ abortSignal: s.viewerSignal.signal } as object),
    });
    s.viewer = viewer;
    linkService.setViewer(viewer);
    eventBus.on("pagesinit", () => {
        if (!s.viewer) return;
        fitToWidth(s);
        if (s.keep) s.viewer.currentPageNumber = Math.min(s.keep.page, s.viewer.pagesCount);
        s.keep = null;
    });
    eventBus.on("pagechanging", (e: { pageNumber: number }) => s.setPage(e.pageNumber));
    eventBus.on("scalechanging", (e: { scale: number }) => s.setScale(e.scale));
    eventBus.on("updatefindmatchescount", (e: { matchesCount: FindMatches }) => s.setMatches(e.matchesCount));
    eventBus.on("updatefindcontrolstate", (e: { matchesCount?: FindMatches }) => {
        if (e.matchesCount) s.setMatches(e.matchesCount);
    });
    return viewer;
}

export async function open(s: PdfViewerState) {
    const mine = ++s.generation;
    if (s.viewer && s.doc) s.keep = { page: s.viewer.currentPageNumber };
    teardown(s);
    s.setStatus("loading");
    s.setError("");
    try {
        const runtime = await import("./pdfjs-runtime");
        s.mods ??= await runtime.loadPdfjs();
        if (mine !== s.generation) return;
        const { pdfjs, viewer: V } = s.mods;
        const viewer = s.viewer ?? createPageViewer(s, s.mods);
        const src = await source(s, pdfjs);
        if (mine !== s.generation) return;
        const task = pdfjs.getDocument({ ...src, ...runtime.SAFE_DOCUMENT_OPTIONS });
        s.loadingTask = task;
        task.onPassword = (update: (password: string) => void, reason: number) => {
            s.sendPassword = update;
            s.setWrongPassword(reason === pdfjs.PasswordResponses.INCORRECT_PASSWORD);
            s.setStatus("password");
            queueMicrotask(() => s.passwordInput?.focus());
        };
        const loaded = await task.promise;
        if (mine !== s.generation) {
            void task.destroy();
            return;
        }
        s.doc = loaded;
        viewer.setDocument(loaded);
        (viewer.linkService as InstanceType<typeof V.PDFLinkService>).setDocument(loaded, null);
        s.setPages(loaded.numPages);
        s.setStatus("ready");
    } catch (e) {
        if (mine !== s.generation) return;
        s.setError(e instanceof Error ? e.message : String(e));
        s.setStatus("error");
    }
}
