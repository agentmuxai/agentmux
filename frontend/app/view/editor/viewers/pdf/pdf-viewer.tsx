// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's PDF viewer: pdf.js's own page viewer (virtualized pages, a
 * selectable text layer, links) inside the Editor's layout, so it works in
 * splits, floating panes and every theme. Scripting, XFA and eval are off
 * (pdfjs-runtime.ts). A large file loads in ranges, so a 300 MB scan opens
 * at page 1 without downloading all of it. External links open in a Browser
 * pane, never in the app; other link schemes are dropped.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.4, §4.7.
 */

import { fetchMediaBlob, fetchMediaRange, formatBytes } from "@/app/element/local-media";
import { Button, IconButton } from "@/app/element/ui";
import { createEffect, createSignal, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import { ViewerBar, type ViewerBarProps } from "../viewer-bar";
import { linkAction } from "./pdf-policy";
import type { PdfjsModules } from "./pdfjs-runtime";
import "./pdf-viewer.scss";

/** Files up to this size load whole; larger ones in ranges. */
export const WHOLE_FILE_MAX_BYTES = 32 * 1024 * 1024;
/** pdf.js's default range size. */
const RANGE_CHUNK = 65536;

export interface PdfViewerProps {
    path: string;
    file?: File;
    revision: number;
    size?: number;
    bar: Omit<ViewerBarProps, "start" | "facts">;
    /** An http(s) link was clicked. */
    onOpenLink: (url: string) => void;
}

type Status = "loading" | "ready" | "password" | "error";

export function PdfViewer(props: PdfViewerProps): JSX.Element {
    const [status, setStatus] = createSignal<Status>("loading");
    const [error, setError] = createSignal("");
    const [page, setPage] = createSignal(1);
    const [pages, setPages] = createSignal(0);
    const [scale, setScale] = createSignal(1);
    const [query, setQuery] = createSignal("");
    const [matches, setMatches] = createSignal<{ current: number; total: number } | null>(null);
    const [wrongPassword, setWrongPassword] = createSignal(false);
    const [invert, setInvert] = createSignal(false);
    let container!: HTMLDivElement;
    let viewerEl!: HTMLDivElement;
    let findInput: HTMLInputElement | undefined;
    let pageInput: HTMLInputElement | undefined;
    let passwordInput: HTMLInputElement | undefined;

    let mods: PdfjsModules | null = null;
    let viewer: InstanceType<PdfjsModules["viewer"]["PDFViewer"]> | null = null;
    let eventBus: InstanceType<PdfjsModules["viewer"]["EventBus"]> | null = null;
    let loadingTask: ReturnType<PdfjsModules["pdfjs"]["getDocument"]> | null = null;
    let doc: Awaited<ReturnType<PdfjsModules["pdfjs"]["getDocument"]>["promise"]> | null = null;
    let sendPassword: ((password: string) => void) | null = null;
    let generation = 0;
    // A reload (the file changed on disk) keeps the page and the zoom.
    let keep: { page: number; scale: string | number } | null = null;

    const teardown = () => {
        sendPassword = null;
        void loadingTask?.destroy().catch(() => {});
        loadingTask = null;
        // Destroying the loading task destroys its document and its worker.
        viewer?.setDocument(null as never);
        doc = null;
    };

    /** The document source: the whole file, or a range transport. */
    const source = async (pdfjs: PdfjsModules["pdfjs"]) => {
        if (props.file) return { data: new Uint8Array(await props.file.arrayBuffer()) };
        if ((props.size ?? 0) <= WHOLE_FILE_MAX_BYTES) {
            return { data: new Uint8Array(await (await fetchMediaBlob(props.path)).arrayBuffer()) };
        }
        const first = await fetchMediaRange(props.path, 0, RANGE_CHUNK - 1);
        if (!first.blob) return { data: new Uint8Array(await (await fetchMediaBlob(props.path)).arrayBuffer()) };
        const range = new pdfjs.PDFDataRangeTransport(first.total, new Uint8Array(await first.blob.arrayBuffer()));
        const path = props.path;
        range.requestDataRange = (begin: number, end: number) => {
            void fetchMediaRange(path, begin, end - 1)
                .then(async (r) => {
                    if (r.blob) range.onDataRange(begin, new Uint8Array(await r.blob.arrayBuffer()));
                })
                .catch(() => {});
        };
        return { range, length: first.total, rangeChunkSize: RANGE_CHUNK, disableAutoFetch: true };
    };

    const open = async () => {
        const mine = ++generation;
        if (viewer && doc) keep = { page: viewer.currentPageNumber, scale: viewer.currentScaleValue };
        teardown();
        setStatus("loading");
        setError("");
        try {
            const runtime = await import("./pdfjs-runtime");
            mods ??= await runtime.loadPdfjs();
            if (mine !== generation) return;
            const { pdfjs, viewer: V } = mods;
            if (!viewer) {
                eventBus = new V.EventBus();
                const linkService = new V.PDFLinkService({ eventBus });
                const findController = new V.PDFFindController({ eventBus, linkService });
                viewer = new V.PDFViewer({
                    container,
                    viewer: viewerEl,
                    eventBus,
                    linkService,
                    findController,
                    textLayerMode: 1,
                    annotationMode: pdfjs.AnnotationMode.ENABLE,
                    // A huge page can't allocate gigabytes of canvas.
                    maxCanvasPixels: 2 ** 25,
                });
                linkService.setViewer(viewer);
                eventBus.on("pagesinit", () => {
                    if (!viewer) return;
                    viewer.currentScaleValue = keep ? String(keep.scale) : "page-width";
                    if (keep) viewer.currentPageNumber = Math.min(keep.page, viewer.pagesCount);
                    keep = null;
                });
                eventBus.on("pagechanging", (e: { pageNumber: number }) => setPage(e.pageNumber));
                eventBus.on("scalechanging", (e: { scale: number }) => setScale(e.scale));
                eventBus.on("updatefindmatchescount", (e: { matchesCount: { current: number; total: number } }) =>
                    setMatches(e.matchesCount)
                );
                eventBus.on("updatefindcontrolstate", (e: { matchesCount?: { current: number; total: number } }) => {
                    if (e.matchesCount) setMatches(e.matchesCount);
                });
            }
            const src = await source(pdfjs);
            if (mine !== generation) return;
            const task = pdfjs.getDocument({ ...src, ...runtime.SAFE_DOCUMENT_OPTIONS });
            loadingTask = task;
            task.onPassword = (update: (password: string) => void, reason: number) => {
                sendPassword = update;
                setWrongPassword(reason === pdfjs.PasswordResponses.INCORRECT_PASSWORD);
                setStatus("password");
                queueMicrotask(() => passwordInput?.focus());
            };
            const loaded = await task.promise;
            if (mine !== generation) {
                void task.destroy();
                return;
            }
            doc = loaded;
            viewer.setDocument(loaded);
            (viewer.linkService as InstanceType<PdfjsModules["viewer"]["PDFLinkService"]>).setDocument(loaded, null);
            setPages(loaded.numPages);
            setStatus("ready");
        } catch (e) {
            if (mine !== generation) return;
            setError(e instanceof Error ? e.message : String(e));
            setStatus("error");
        }
    };

    onMount(() => {
        // Links: internal ones are pdf.js's; an external one opens a Browser
        // pane or is dropped, and never navigates the app.
        const onClick = (e: MouseEvent) => {
            const a = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
            if (!a) return;
            const href = a.getAttribute("href") ?? "";
            if (href.startsWith("#")) return;
            e.preventDefault();
            e.stopPropagation();
            if (linkAction(a.href) === "browser") props.onOpenLink(a.href);
        };
        container.addEventListener("click", onClick, true);
        onCleanup(() => container.removeEventListener("click", onClick, true));
        void open();
    });
    // The file changed on disk, or the tab shows another one.
    createEffect(on(() => [props.path, props.revision, props.file] as const, () => void open(), { defer: true }));
    onCleanup(() => {
        generation++;
        teardown();
        viewer = null;
        eventBus = null;
    });

    const goTo = (n: number) => {
        if (!viewer || !pages()) return;
        viewer.currentPageNumber = Math.max(1, Math.min(pages(), Math.round(n)));
    };
    const zoom = (how: "in" | "out" | "page-width" | "page-fit") => {
        if (!viewer) return;
        if (how === "in") viewer.increaseScale();
        else if (how === "out") viewer.decreaseScale();
        else viewer.currentScaleValue = how;
    };
    const find = (again: boolean, previous = false) => {
        if (!eventBus) return;
        eventBus.dispatch("find", {
            source: null,
            type: again ? "again" : "",
            query: query(),
            caseSensitive: false,
            entireWord: false,
            highlightAll: true,
            findPrevious: previous,
            matchDiacritics: false,
        });
    };
    const onKeyDown = (e: KeyboardEvent) => {
        const mod = e.ctrlKey || e.metaKey;
        if (mod && (e.key === "f" || e.key === "F")) {
            e.preventDefault();
            e.stopPropagation();
            findInput?.focus();
            findInput?.select();
        } else if (mod && (e.key === "g" || e.key === "G")) {
            e.preventDefault();
            e.stopPropagation();
            pageInput?.focus();
            pageInput?.select();
        } else if (mod && (e.key === "=" || e.key === "+")) {
            e.preventDefault();
            e.stopPropagation();
            zoom("in");
        } else if (mod && e.key === "-") {
            e.preventDefault();
            e.stopPropagation();
            zoom("out");
        } else if (!mod && e.key === "Home" && e.target === container) {
            e.preventDefault();
            goTo(1);
        } else if (!mod && e.key === "End" && e.target === container) {
            e.preventDefault();
            goTo(pages());
        }
    };

    const facts = (): string[] => {
        const out: string[] = [];
        if (pages()) out.push(`${pages()} page${pages() === 1 ? "" : "s"}`);
        const size = props.size ?? props.file?.size;
        if (size != null) out.push(formatBytes(size));
        out.push("PDF");
        return out;
    };

    return (
        <div class="editor-viewer pdf-viewer" onKeyDown={onKeyDown}>
            <ViewerBar
                {...props.bar}
                facts={facts()}
                start={
                    <>
                        <IconButton icon="chevron-up" density="compact" label="Previous page" disabled={page() <= 1} onClick={() => goTo(page() - 1)} />
                        <input
                            ref={pageInput}
                            class="pdf-viewer-page-input"
                            aria-label="Page"
                            title="Page (Ctrl+G)"
                            value={page()}
                            onChange={(e) => goTo(Number(e.currentTarget.value) || page())}
                            onKeyDown={(e) => e.key === "Enter" && goTo(Number(e.currentTarget.value) || page())}
                        />
                        <span class="pdf-viewer-of">/ {pages() || "–"}</span>
                        <IconButton icon="chevron-down" density="compact" label="Next page" disabled={page() >= pages()} onClick={() => goTo(page() + 1)} />
                        <IconButton icon="magnifying-glass-minus" density="compact" label="Zoom out (Ctrl+−)" onClick={() => zoom("out")} />
                        <span class="pdf-viewer-scale" aria-label="Zoom">
                            {Math.round(scale() * 100)}%
                        </span>
                        <IconButton icon="magnifying-glass-plus" density="compact" label="Zoom in (Ctrl+=)" onClick={() => zoom("in")} />
                        <Button density="compact" title="Fit the page width" onClick={() => zoom("page-width")}>
                            Width
                        </Button>
                        <Button density="compact" title="Fit the whole page" onClick={() => zoom("page-fit")}>
                            Page
                        </Button>
                        <IconButton
                            icon="circle-half-stroke"
                            density="compact"
                            pressed={invert()}
                            label={invert() ? "Show the pages as they are" : "Invert the pages (for a dark theme)"}
                            onClick={() => setInvert(!invert())}
                        />
                        <input
                            ref={findInput}
                            class="pdf-viewer-find"
                            type="search"
                            placeholder="Find"
                            aria-label="Find in the document"
                            title="Find (Ctrl+F); Enter for the next match, Shift+Enter for the previous"
                            value={query()}
                            onInput={(e) => {
                                setQuery(e.currentTarget.value);
                                find(false);
                            }}
                            onKeyDown={(e) => {
                                if (e.key === "Enter") {
                                    e.preventDefault();
                                    find(true, e.shiftKey);
                                }
                            }}
                        />
                        <Show when={query() && matches()}>
                            <span class="pdf-viewer-matches" role="status">
                                {matches()!.total ? `${matches()!.current} of ${matches()!.total}` : "No matches"}
                            </span>
                        </Show>
                    </>
                }
            />
            <div class="pdf-viewer-body">
                <div ref={container} class="pdf-viewer-container" classList={{ "pdf-viewer-inverted": invert() }} tabIndex={0} aria-label="PDF pages">
                    <div ref={viewerEl} class="pdfViewer" />
                </div>
                <Show when={status() === "loading"}>
                    <div class="editor-viewer-message">Loading…</div>
                </Show>
                <Show when={status() === "error"}>
                    <div class="editor-viewer-message" role="alert">
                        Couldn't open this PDF: {error()}
                    </div>
                </Show>
                <Show when={status() === "password"}>
                    <form
                        class="editor-viewer-message pdf-viewer-password"
                        onSubmit={(e) => {
                            e.preventDefault();
                            const pw = passwordInput?.value ?? "";
                            setStatus("loading");
                            sendPassword?.(pw);
                        }}
                    >
                        <div>{wrongPassword() ? "That password didn't open it. Try again:" : "This PDF is protected. Its password:"}</div>
                        <input ref={passwordInput} type="password" aria-label="PDF password" autocomplete="off" />
                        <Button type="submit" tone="accent">
                            Open
                        </Button>
                    </form>
                </Show>
            </div>
        </div>
    );
}
